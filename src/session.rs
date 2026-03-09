use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    error::{NavError, OpError, ValidationError},
    geom::{Axis, Direction, Rect, Slot},
    ids::{NodeId, Revision},
    limits::{LeafMeta, Summary, WeightPair, canonicalize_weights},
    nav::best_neighbor,
    preset::{BalancedPreset, PresetKind, TallPreset, WidePreset},
    resize::ResizeStrategy,
    snapshot::Snapshot,
    solver::{SolverPolicy, summarize},
    tree::{Node, SplitNode, Tree},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebalanceMode {
    BinaryEqual,
    LeafCount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session<T> {
    pub tree: Tree<T>,
    pub focus: Option<NodeId>,
    pub selection: Option<NodeId>,
    pub revision: Revision,
}

impl<T> Default for Session<T> {
    fn default() -> Self {
        Self {
            tree: Tree::default(),
            focus: None,
            selection: None,
            revision: 0,
        }
    }
}

impl<T> Session<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        self.tree.validate()?;
        match self.tree.root {
            None => {
                if self.focus.is_none() && self.selection.is_none() {
                    Ok(())
                } else {
                    Err(ValidationError::EmptyStateInconsistent)
                }
            }
            Some(_) => {
                let focus = self.focus.ok_or(ValidationError::EmptyStateInconsistent)?;
                if !self.tree.is_leaf(focus) {
                    return Err(ValidationError::NonLeafFocus(focus));
                }
                let selection = self
                    .selection
                    .ok_or(ValidationError::EmptyStateInconsistent)?;
                if !self.tree.contains(selection) {
                    return Err(ValidationError::InvalidSelection(selection));
                }
                if self.tree.is_leaf(selection) {
                    if selection == focus {
                        Ok(())
                    } else {
                        Err(ValidationError::InvalidSelection(selection))
                    }
                } else if self.tree.contains_in_subtree(selection, focus) {
                    Ok(())
                } else {
                    Err(ValidationError::InvalidSelection(selection))
                }
            }
        }
    }

    #[must_use]
    pub fn solve(&self, root: Rect, policy: &SolverPolicy) -> Snapshot {
        crate::solver::solve_with_revision(&self.tree, root, self.revision, policy)
            .expect("session should maintain a valid tree")
    }

    pub fn insert_root(&mut self, payload: T, meta: LeafMeta) -> Result<NodeId, OpError> {
        if self.tree.root.is_some() {
            return Err(OpError::NonEmpty);
        }
        let id = self.tree.new_leaf(payload, meta);
        self.tree.root = Some(id);
        self.focus = Some(id);
        self.selection = Some(id);
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(id)
    }

    pub fn split_focus(
        &mut self,
        axis: Axis,
        slot: Slot,
        payload: T,
        meta: LeafMeta,
        weights: Option<WeightPair>,
    ) -> Result<NodeId, OpError> {
        let focus = self.focus_leaf()?;
        let weights = checked_weights(weights.unwrap_or_default())?;
        let old_selection = self.selection;
        let new_leaf = self.tree.new_leaf(payload, meta);
        let split_id = self.wrap_existing_with_leaf(focus, axis, slot, new_leaf, weights);
        self.repair_after_mutation(focus, old_selection, Some(split_id));
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(new_leaf)
    }

    pub fn wrap_selection(
        &mut self,
        axis: Axis,
        slot: Slot,
        payload: T,
        meta: LeafMeta,
        weights: Option<WeightPair>,
    ) -> Result<NodeId, OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        let focus = self.focus_leaf()?;
        let old_selection = self.selection;
        let new_leaf = self.tree.new_leaf(payload, meta);
        let split_id = self.wrap_existing_with_leaf(
            selection,
            axis,
            slot,
            new_leaf,
            checked_weights(weights.unwrap_or_default())?,
        );
        self.repair_after_mutation(focus, old_selection, Some(split_id));
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(new_leaf)
    }

    pub fn remove_focus(&mut self) -> Result<(), OpError> {
        let focus = self.focus_leaf()?;
        let old_selection = self.selection;
        let fallback = if self.tree.root == Some(focus) {
            self.tree.root = None;
            None
        } else {
            let sibling = self
                .tree
                .sibling_of(focus)
                .ok_or(OpError::NoParent(focus))?;
            let replacement = self.tree.collapse_unary_parent(focus).unwrap_or(sibling);
            Some(replacement)
        };
        self.tree.nodes.remove(&focus);
        self.repair_after_mutation(focus, old_selection, fallback);
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(())
    }

    pub fn swap_nodes(&mut self, a: NodeId, b: NodeId) -> Result<(), OpError> {
        if a == b {
            return Err(OpError::SameNode);
        }
        self.require_node(a)?;
        self.require_node(b)?;
        if self.tree.contains_in_subtree(a, b) || self.tree.contains_in_subtree(b, a) {
            return Err(OpError::AncestorConflict);
        }
        let focus = self.focus_leaf()?;
        let old_selection = self.selection;
        let parent_a = self.tree.parent_of(a);
        let parent_b = self.tree.parent_of(b);
        if parent_a == parent_b {
            let parent = parent_a.ok_or(OpError::AncestorConflict)?;
            let split = self.split_mut(parent)?;
            if split.a == a && split.b == b {
                split.a = b;
                split.b = a;
            } else if split.a == b && split.b == a {
                split.a = a;
                split.b = b;
            } else {
                return Err(OpError::AncestorConflict);
            }
        } else {
            match parent_a {
                Some(parent) => self.tree.replace_child(parent, a, b),
                None => {
                    self.tree.root = Some(b);
                    self.tree.set_parent(b, None);
                }
            }
            match parent_b {
                Some(parent) => self.tree.replace_child(parent, b, a),
                None => {
                    self.tree.root = Some(a);
                    self.tree.set_parent(a, None);
                }
            }
            self.tree.set_parent(a, parent_b);
            self.tree.set_parent(b, parent_a);
        }
        self.repair_after_mutation(focus, old_selection, self.tree.root);
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(())
    }

    pub fn move_selection_as_sibling_of(
        &mut self,
        target: NodeId,
        axis: Axis,
        slot: Slot,
    ) -> Result<(), OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        if selection == target {
            return Err(OpError::SameNode);
        }
        self.require_node(target)?;
        if self.tree.contains_in_subtree(selection, target) {
            return Err(OpError::TargetInsideSelection);
        }
        let focus = self.focus_leaf()?;
        let old_selection = self.selection;
        let effective_target = remaining_subtree_after_detach(&self.tree, target, selection)
            .ok_or(OpError::AncestorConflict)?;
        self.tree.detach_subtree(selection);
        let split_id = self.tree.attach_as_sibling(
            effective_target,
            selection,
            axis,
            slot,
            WeightPair::default(),
        );
        self.repair_after_mutation(focus, old_selection, Some(split_id));
        self.bump_revision();
        self.validate().map_err(OpError::Validation)?;
        Ok(())
    }

    pub fn focus_dir(&mut self, dir: Direction, snap: &Snapshot) -> Result<(), NavError> {
        self.ensure_fresh_snapshot(snap).map_err(map_op_to_nav)?;
        let focus = self.focus.ok_or(NavError::Empty)?;
        let leaf_rects = self.leaf_rects_from_snapshot(snap)?;
        let next =
            best_neighbor(&self.tree, &leaf_rects, focus, dir).ok_or(NavError::NoCandidate)?;
        self.focus = Some(next);
        self.repair_selection_for_current_focus();
        self.validate().map_err(NavError::Validation)
    }

    pub fn select_parent(&mut self) -> Result<(), OpError> {
        let base = self.selection.or(self.focus).ok_or(OpError::Empty)?;
        let parent = self.tree.parent_of(base).ok_or(OpError::NoParent(base))?;
        self.selection = Some(parent);
        self.validate().map_err(OpError::Validation)
    }

    pub fn select_focus(&mut self) {
        self.selection = self.focus;
    }

    pub fn grow_focus(
        &mut self,
        dir: Direction,
        amount: u32,
        strategy: ResizeStrategy,
        snap: &Snapshot,
    ) -> Result<(), OpError> {
        self.resize_focus(dir, amount, strategy, snap, true)
    }

    pub fn shrink_focus(
        &mut self,
        dir: Direction,
        amount: u32,
        strategy: ResizeStrategy,
        snap: &Snapshot,
    ) -> Result<(), OpError> {
        self.resize_focus(dir, amount, strategy, snap, false)
    }

    pub fn toggle_axis(&mut self) -> Result<(), OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        let split = self.split_mut(selection)?;
        split.axis = toggle_axis(split.axis);
        self.bump_revision();
        self.validate().map_err(OpError::Validation)
    }

    pub fn mirror_selection(&mut self, axis: Axis) -> Result<(), OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        self.mirror_subtree(selection, axis)?;
        self.bump_revision();
        self.validate().map_err(OpError::Validation)
    }

    pub fn rebalance_selection(&mut self, mode: RebalanceMode) -> Result<(), OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        self.rebalance_subtree(selection, mode)?;
        self.bump_revision();
        self.validate().map_err(OpError::Validation)
    }

    pub fn apply_preset(&mut self, preset: PresetKind) -> Result<(), OpError> {
        let selection = self.selection.ok_or(OpError::Empty)?;
        if self.tree.is_leaf(selection) {
            return Ok(());
        }
        if self.subtree_matches_preset(selection, preset)? {
            return Ok(());
        }
        let focus = self.focus_leaf()?;
        let old_selection = self.selection;
        let parent = self.tree.parent_of(selection);
        let leaves = self.tree.leaf_ids_dfs(selection);
        let split_ids = collect_split_ids(&self.tree, selection);
        for leaf in &leaves {
            self.tree.set_parent(*leaf, None);
        }
        for split in split_ids {
            self.tree.nodes.remove(&split);
        }
        let rebuilt = self.build_preset_subtree(&leaves, preset)?;
        match parent {
            Some(parent) => {
                self.tree.replace_child(parent, selection, rebuilt);
                self.tree.set_parent(rebuilt, Some(parent));
            }
            None => {
                self.tree.root = Some(rebuilt);
                self.tree.set_parent(rebuilt, None);
            }
        }
        self.repair_after_mutation(focus, old_selection, Some(rebuilt));
        self.bump_revision();
        self.validate().map_err(OpError::Validation)
    }

    fn resize_focus(
        &mut self,
        dir: Direction,
        amount: u32,
        strategy: ResizeStrategy,
        snap: &Snapshot,
        outward: bool,
    ) -> Result<(), OpError> {
        if amount == 0 {
            return Ok(());
        }
        self.ensure_fresh_snapshot(snap)?;
        let focus = self.focus_leaf()?;
        let mut summaries = HashMap::new();
        if let Some(root) = self.tree.root {
            summarize(&self.tree, root, &mut summaries).map_err(OpError::Validation)?;
        }
        let eligible = self.eligible_splits(focus, dir, snap, &summaries)?;
        if eligible.is_empty() {
            return Ok(());
        }
        let sign = resize_sign(dir, outward);
        let allocations = distribute_resize(amount, strategy, sign, &eligible);
        for (split_id, delta) in allocations {
            if delta == 0 {
                continue;
            }
            let info = eligible
                .iter()
                .find(|entry| entry.split == split_id)
                .expect("eligible split missing during resize");
            let new_a = if sign > 0 {
                info.current_a + delta
            } else {
                info.current_a - delta
            };
            let total = info.total;
            let weights = canonicalize_weights(new_a, total - new_a);
            self.split_mut(split_id)?.weights = weights;
        }
        self.bump_revision();
        self.validate().map_err(OpError::Validation)
    }

    fn leaf_rects_from_snapshot(&self, snap: &Snapshot) -> Result<HashMap<NodeId, Rect>, NavError> {
        self.tree
            .root
            .map(|root| self.tree.leaf_ids_dfs(root))
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                snap.rect(id)
                    .map(|rect| (id, rect))
                    .ok_or(NavError::MissingSnapshotRect(id))
            })
            .collect()
    }

    fn eligible_splits(
        &self,
        focus: NodeId,
        dir: Direction,
        snap: &Snapshot,
        summaries: &HashMap<NodeId, Summary>,
    ) -> Result<Vec<EligibleSplit>, OpError> {
        let focus_rect = snap.rect(focus).ok_or(OpError::MissingNode(focus))?;
        let mut out = Vec::new();
        for split_id in self.tree.ancestors_nearest_first(focus) {
            let split = self
                .tree
                .nodes
                .get(&split_id)
                .and_then(Node::as_split)
                .ok_or(OpError::NotSplit(split_id))?;
            let a_rect = snap.rect(split.a).ok_or(OpError::MissingNode(split.a))?;
            let b_rect = snap.rect(split.b).ok_or(OpError::MissingNode(split.b))?;
            let focus_in_a = self.tree.contains_in_subtree(split.a, focus);
            let eligible = match dir {
                Direction::Right => {
                    split.axis == Axis::X && focus_in_a && focus_rect.right() == a_rect.right()
                }
                Direction::Left => {
                    split.axis == Axis::X && !focus_in_a && focus_rect.left() == b_rect.left()
                }
                Direction::Down => {
                    split.axis == Axis::Y && focus_in_a && focus_rect.bottom() == a_rect.bottom()
                }
                Direction::Up => {
                    split.axis == Axis::Y && !focus_in_a && focus_rect.top() == b_rect.top()
                }
            };
            if !eligible {
                continue;
            }
            let total = a_rect.extent(split.axis) + b_rect.extent(split.axis);
            let sum_a = summaries
                .get(&split.a)
                .copied()
                .ok_or(OpError::MissingNode(split.a))?;
            let sum_b = summaries
                .get(&split.b)
                .copied()
                .ok_or(OpError::MissingNode(split.b))?;
            let (min_a, max_a) = sum_a.axis_limits(split.axis);
            let (min_b, max_b) = sum_b.axis_limits(split.axis);
            let lo = min_a.max(max_b.map_or(0, |max_b| total.saturating_sub(max_b)));
            let hi = total.saturating_sub(min_b).min(max_a.unwrap_or(total));
            let current_a = a_rect.extent(split.axis);
            out.push(EligibleSplit {
                split: split_id,
                total,
                current_a,
                lo,
                hi,
            });
        }
        Ok(out)
    }

    fn build_preset_subtree(
        &mut self,
        leaves: &[NodeId],
        preset: PresetKind,
    ) -> Result<NodeId, OpError> {
        match preset {
            PresetKind::Balanced(preset) => self.build_balanced(leaves, preset),
            PresetKind::Dwindle(preset) => {
                self.build_dwindle(leaves, preset.start_axis, preset.new_leaf_slot)
            }
            PresetKind::Tall(preset) => self.build_tall(leaves, preset),
            PresetKind::Wide(preset) => self.build_wide(leaves, preset),
        }
    }

    fn subtree_matches_preset(&self, root: NodeId, preset: PresetKind) -> Result<bool, OpError> {
        let leaves = self.tree.leaf_ids_dfs(root);
        match preset {
            PresetKind::Balanced(preset) => Ok(self.matches_balanced(root, &leaves, preset)),
            PresetKind::Dwindle(preset) => {
                Ok(self.matches_dwindle(root, &leaves, preset.start_axis, preset.new_leaf_slot))
            }
            PresetKind::Tall(preset) => Ok(self.matches_tall(root, &leaves, preset)),
            PresetKind::Wide(preset) => Ok(self.matches_wide(root, &leaves, preset)),
        }
    }

    fn build_balanced(
        &mut self,
        leaves: &[NodeId],
        preset: BalancedPreset,
    ) -> Result<NodeId, OpError> {
        if leaves.is_empty() {
            return Err(OpError::Empty);
        }
        if leaves.len() == 1 {
            return Ok(leaves[0]);
        }
        let mid = leaves.len().div_ceil(2);
        let next_axis = if preset.alternate {
            toggle_axis(preset.start_axis)
        } else {
            preset.start_axis
        };
        let a = self.build_balanced(
            &leaves[..mid],
            BalancedPreset {
                start_axis: next_axis,
                alternate: preset.alternate,
            },
        )?;
        let b = self.build_balanced(
            &leaves[mid..],
            BalancedPreset {
                start_axis: next_axis,
                alternate: preset.alternate,
            },
        )?;
        Ok(self.new_internal_split(
            preset.start_axis,
            a,
            b,
            canonicalize_weights(mid as u32, (leaves.len() - mid) as u32),
        ))
    }

    fn build_dwindle(
        &mut self,
        leaves: &[NodeId],
        axis: Axis,
        slot: Slot,
    ) -> Result<NodeId, OpError> {
        if leaves.is_empty() {
            return Err(OpError::Empty);
        }
        if leaves.len() == 1 {
            return Ok(leaves[0]);
        }
        let first = leaves[0];
        let rest = self.build_dwindle(&leaves[1..], toggle_axis(axis), slot)?;
        let (a, b) = match slot {
            Slot::A => (rest, first),
            Slot::B => (first, rest),
        };
        Ok(self.new_internal_split(axis, a, b, WeightPair::default()))
    }

    fn build_tall(&mut self, leaves: &[NodeId], preset: TallPreset) -> Result<NodeId, OpError> {
        if leaves.is_empty() {
            return Err(OpError::Empty);
        }
        if leaves.len() == 1 {
            return Ok(leaves[0]);
        }
        let master = leaves[0];
        let stack = self.build_equal_linear(&leaves[1..], Axis::Y)?;
        let (a, b) = match preset.master_slot {
            Slot::A => (master, stack),
            Slot::B => (stack, master),
        };
        Ok(self.new_internal_split(Axis::X, a, b, checked_weights(preset.root_weights)?))
    }

    fn build_wide(&mut self, leaves: &[NodeId], preset: WidePreset) -> Result<NodeId, OpError> {
        if leaves.is_empty() {
            return Err(OpError::Empty);
        }
        if leaves.len() == 1 {
            return Ok(leaves[0]);
        }
        let master = leaves[0];
        let stack = self.build_equal_linear(&leaves[1..], Axis::X)?;
        let (a, b) = match preset.master_slot {
            Slot::A => (master, stack),
            Slot::B => (stack, master),
        };
        Ok(self.new_internal_split(Axis::Y, a, b, checked_weights(preset.root_weights)?))
    }

    fn build_equal_linear(&mut self, leaves: &[NodeId], axis: Axis) -> Result<NodeId, OpError> {
        if leaves.is_empty() {
            return Err(OpError::Empty);
        }
        if leaves.len() == 1 {
            return Ok(leaves[0]);
        }
        let head = leaves[0];
        let rest = self.build_equal_linear(&leaves[1..], axis)?;
        Ok(self.new_internal_split(
            axis,
            head,
            rest,
            canonicalize_weights(1, (leaves.len() - 1) as u32),
        ))
    }

    fn matches_balanced(&self, id: NodeId, leaves: &[NodeId], preset: BalancedPreset) -> bool {
        if leaves.is_empty() {
            return false;
        }
        if leaves.len() == 1 {
            return self.tree.is_leaf(id) && id == leaves[0];
        }
        let Some(split) = self.tree.nodes.get(&id).and_then(Node::as_split) else {
            return false;
        };
        if split.axis != preset.start_axis {
            return false;
        }
        let mid = leaves.len().div_ceil(2);
        if split.weights != canonicalize_weights(mid as u32, (leaves.len() - mid) as u32) {
            return false;
        }
        let next = BalancedPreset {
            start_axis: if preset.alternate {
                toggle_axis(preset.start_axis)
            } else {
                preset.start_axis
            },
            alternate: preset.alternate,
        };
        self.matches_balanced(split.a, &leaves[..mid], next)
            && self.matches_balanced(split.b, &leaves[mid..], next)
    }

    fn matches_dwindle(&self, id: NodeId, leaves: &[NodeId], axis: Axis, slot: Slot) -> bool {
        if leaves.is_empty() {
            return false;
        }
        if leaves.len() == 1 {
            return self.tree.is_leaf(id) && id == leaves[0];
        }
        let Some(split) = self.tree.nodes.get(&id).and_then(Node::as_split) else {
            return false;
        };
        if split.axis != axis || split.weights != WeightPair::default() {
            return false;
        }
        match slot {
            Slot::A => {
                split.b == leaves[0]
                    && self.tree.is_leaf(split.b)
                    && self.matches_dwindle(split.a, &leaves[1..], toggle_axis(axis), slot)
            }
            Slot::B => {
                split.a == leaves[0]
                    && self.tree.is_leaf(split.a)
                    && self.matches_dwindle(split.b, &leaves[1..], toggle_axis(axis), slot)
            }
        }
    }

    fn matches_tall(&self, id: NodeId, leaves: &[NodeId], preset: TallPreset) -> bool {
        if leaves.is_empty() {
            return false;
        }
        if leaves.len() == 1 {
            return self.tree.is_leaf(id) && id == leaves[0];
        }
        let Some(split) = self.tree.nodes.get(&id).and_then(Node::as_split) else {
            return false;
        };
        if split.axis != Axis::X || split.weights != preset.root_weights {
            return false;
        }
        match preset.master_slot {
            Slot::A => {
                split.a == leaves[0]
                    && self.tree.is_leaf(split.a)
                    && self.matches_equal_linear(split.b, &leaves[1..], Axis::Y)
            }
            Slot::B => {
                split.b == leaves[0]
                    && self.tree.is_leaf(split.b)
                    && self.matches_equal_linear(split.a, &leaves[1..], Axis::Y)
            }
        }
    }

    fn matches_wide(&self, id: NodeId, leaves: &[NodeId], preset: WidePreset) -> bool {
        if leaves.is_empty() {
            return false;
        }
        if leaves.len() == 1 {
            return self.tree.is_leaf(id) && id == leaves[0];
        }
        let Some(split) = self.tree.nodes.get(&id).and_then(Node::as_split) else {
            return false;
        };
        if split.axis != Axis::Y || split.weights != preset.root_weights {
            return false;
        }
        match preset.master_slot {
            Slot::A => {
                split.a == leaves[0]
                    && self.tree.is_leaf(split.a)
                    && self.matches_equal_linear(split.b, &leaves[1..], Axis::X)
            }
            Slot::B => {
                split.b == leaves[0]
                    && self.tree.is_leaf(split.b)
                    && self.matches_equal_linear(split.a, &leaves[1..], Axis::X)
            }
        }
    }

    fn matches_equal_linear(&self, id: NodeId, leaves: &[NodeId], axis: Axis) -> bool {
        if leaves.is_empty() {
            return false;
        }
        if leaves.len() == 1 {
            return self.tree.is_leaf(id) && id == leaves[0];
        }
        let Some(split) = self.tree.nodes.get(&id).and_then(Node::as_split) else {
            return false;
        };
        split.axis == axis
            && split.weights == canonicalize_weights(1, (leaves.len() - 1) as u32)
            && split.a == leaves[0]
            && self.tree.is_leaf(split.a)
            && self.matches_equal_linear(split.b, &leaves[1..], axis)
    }

    fn rebalance_subtree(&mut self, id: NodeId, mode: RebalanceMode) -> Result<u32, OpError> {
        if self.tree.is_leaf(id) {
            return Ok(1);
        }
        let (a, b) = self.tree.children_of(id).ok_or(OpError::NotSplit(id))?;
        let count_a = self.rebalance_subtree(a, mode)?;
        let count_b = self.rebalance_subtree(b, mode)?;
        let weights = match mode {
            RebalanceMode::BinaryEqual => WeightPair::default(),
            RebalanceMode::LeafCount => canonicalize_weights(count_a, count_b),
        };
        self.split_mut(id)?.weights = weights;
        Ok(count_a + count_b)
    }

    fn mirror_subtree(&mut self, id: NodeId, axis: Axis) -> Result<(), OpError> {
        let children = match self.tree.children_of(id) {
            Some(children) => children,
            None => return Ok(()),
        };
        self.mirror_subtree(children.0, axis)?;
        self.mirror_subtree(children.1, axis)?;
        let split = self.split_mut(id)?;
        if split.axis == axis {
            std::mem::swap(&mut split.a, &mut split.b);
            std::mem::swap(&mut split.weights.a, &mut split.weights.b);
        }
        Ok(())
    }

    fn new_internal_split(
        &mut self,
        axis: Axis,
        a: NodeId,
        b: NodeId,
        weights: WeightPair,
    ) -> NodeId {
        let split_id = self.tree.new_split(axis, a, b, weights);
        self.tree.set_parent(a, Some(split_id));
        self.tree.set_parent(b, Some(split_id));
        split_id
    }

    fn wrap_existing_with_leaf(
        &mut self,
        existing: NodeId,
        axis: Axis,
        slot: Slot,
        new_leaf: NodeId,
        weights: WeightPair,
    ) -> NodeId {
        let parent = self.tree.parent_of(existing);
        let (a, b) = match slot {
            Slot::A => (new_leaf, existing),
            Slot::B => (existing, new_leaf),
        };
        let split_id = self.tree.new_split(axis, a, b, weights);
        self.tree.set_parent(a, Some(split_id));
        self.tree.set_parent(b, Some(split_id));
        match parent {
            Some(parent) => {
                self.tree.replace_child(parent, existing, split_id);
                self.tree.set_parent(split_id, Some(parent));
            }
            None => {
                self.tree.root = Some(split_id);
                self.tree.set_parent(split_id, None);
            }
        }
        split_id
    }

    fn repair_after_mutation(
        &mut self,
        old_focus: NodeId,
        old_selection: Option<NodeId>,
        replacement_site: Option<NodeId>,
    ) {
        self.focus = if self.tree.root.is_none() {
            None
        } else if self.tree.is_leaf(old_focus) {
            Some(old_focus)
        } else {
            replacement_site.and_then(|id| self.tree.first_leaf(id))
        };

        self.selection = match (self.tree.root, self.focus) {
            (None, _) | (_, None) => None,
            (Some(_), Some(focus)) => old_selection
                .filter(|selection| self.tree.contains(*selection))
                .filter(|selection| {
                    if self.tree.is_leaf(*selection) {
                        *selection == focus
                    } else {
                        self.tree.contains_in_subtree(*selection, focus)
                    }
                })
                .or(Some(focus)),
        };
    }

    fn repair_selection_for_current_focus(&mut self) {
        self.selection = match (self.selection, self.focus) {
            (_, None) => None,
            (Some(selection), Some(focus)) if self.tree.contains(selection) => {
                if self.tree.is_leaf(selection) {
                    Some(focus)
                } else if self.tree.contains_in_subtree(selection, focus) {
                    Some(selection)
                } else {
                    Some(focus)
                }
            }
            (None, Some(focus)) => Some(focus),
            (Some(_), Some(focus)) => Some(focus),
        };
    }

    fn ensure_fresh_snapshot(&self, snap: &Snapshot) -> Result<(), OpError> {
        if snap.revision == self.revision {
            Ok(())
        } else {
            Err(OpError::StaleSnapshot)
        }
    }

    fn focus_leaf(&self) -> Result<NodeId, OpError> {
        let focus = self.focus.ok_or(OpError::Empty)?;
        if self.tree.is_leaf(focus) {
            Ok(focus)
        } else {
            Err(OpError::NotLeaf(focus))
        }
    }

    fn split_mut(&mut self, id: NodeId) -> Result<&mut SplitNode, OpError> {
        self.tree
            .nodes
            .get_mut(&id)
            .and_then(Node::as_split_mut)
            .ok_or(OpError::NotSplit(id))
    }

    fn require_node(&self, id: NodeId) -> Result<(), OpError> {
        if self.tree.contains(id) {
            Ok(())
        } else {
            Err(OpError::MissingNode(id))
        }
    }

    fn bump_revision(&mut self) {
        self.revision += 1;
    }
}

#[derive(Debug, Clone, Copy)]
struct EligibleSplit {
    split: NodeId,
    total: u32,
    current_a: u32,
    lo: u32,
    hi: u32,
}

fn distribute_resize(
    amount: u32,
    strategy: ResizeStrategy,
    sign: i8,
    eligible: &[EligibleSplit],
) -> Vec<(NodeId, u32)> {
    match strategy {
        ResizeStrategy::Local => eligible
            .first()
            .map(|entry| (entry.split, amount.min(entry.slack(sign))))
            .into_iter()
            .collect(),
        ResizeStrategy::AncestorChain => {
            let mut remaining = amount;
            let mut out = Vec::new();
            for entry in eligible {
                if remaining == 0 {
                    break;
                }
                let delta = remaining.min(entry.slack(sign));
                if delta != 0 {
                    out.push((entry.split, delta));
                    remaining -= delta;
                }
            }
            out
        }
        ResizeStrategy::DistributedBySlack => {
            let total_slack = eligible.iter().map(|entry| entry.slack(sign)).sum::<u32>();
            if total_slack == 0 {
                return Vec::new();
            }
            let request = amount.min(total_slack);
            let mut assigned = 0_u32;
            let mut allocations = eligible
                .iter()
                .enumerate()
                .map(|(idx, entry)| {
                    let slack = entry.slack(sign);
                    let product = u128::from(request) * u128::from(slack);
                    let base = u32::try_from(product / u128::from(total_slack))
                        .expect("base resize share exceeds u32");
                    let remainder = u32::try_from(product % u128::from(total_slack))
                        .expect("remainder exceeds u32");
                    assigned += base;
                    (idx, entry.split, slack, base.min(slack), remainder)
                })
                .collect::<Vec<_>>();
            let mut leftover = request - assigned;
            allocations
                .sort_by_key(|(idx, _, _, _, remainder)| (std::cmp::Reverse(*remainder), *idx));
            for (_, _, slack, base, _) in &mut allocations {
                if leftover == 0 {
                    break;
                }
                if *base < *slack {
                    *base += 1;
                    leftover -= 1;
                }
            }
            allocations.sort_by_key(|(idx, ..)| *idx);
            allocations
                .into_iter()
                .filter_map(|(_, split, _, base, _)| (base != 0).then_some((split, base)))
                .collect()
        }
    }
}

impl EligibleSplit {
    fn slack(self, sign: i8) -> u32 {
        if self.lo > self.hi {
            return 0;
        }
        if sign > 0 {
            self.hi.saturating_sub(self.current_a)
        } else {
            self.current_a.saturating_sub(self.lo)
        }
    }
}

fn collect_split_ids<T>(tree: &Tree<T>, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    collect_split_ids_inner(tree, id, &mut out);
    out
}

fn collect_split_ids_inner<T>(tree: &Tree<T>, id: NodeId, out: &mut Vec<NodeId>) {
    if let Some((a, b)) = tree.children_of(id) {
        collect_split_ids_inner(tree, a, out);
        collect_split_ids_inner(tree, b, out);
        out.push(id);
    }
}

fn checked_weights(weights: WeightPair) -> Result<WeightPair, OpError> {
    if weights.a == 0 && weights.b == 0 {
        Err(OpError::InvalidWeights)
    } else {
        Ok(weights)
    }
}

fn remaining_subtree_after_detach<T>(
    tree: &Tree<T>,
    current: NodeId,
    removed: NodeId,
) -> Option<NodeId> {
    if current == removed {
        return None;
    }
    match tree.nodes.get(&current)? {
        Node::Leaf(_) => Some(current),
        Node::Split(split) => match (
            remaining_subtree_after_detach(tree, split.a, removed),
            remaining_subtree_after_detach(tree, split.b, removed),
        ) {
            (Some(_), Some(_)) => Some(current),
            (Some(id), None) | (None, Some(id)) => Some(id),
            (None, None) => None,
        },
    }
}

fn toggle_axis(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

fn resize_sign(dir: Direction, outward: bool) -> i8 {
    match (dir, outward) {
        (Direction::Right | Direction::Down, true) | (Direction::Left | Direction::Up, false) => 1,
        (Direction::Left | Direction::Up, true) | (Direction::Right | Direction::Down, false) => -1,
    }
}

fn map_op_to_nav(error: OpError) -> NavError {
    match error {
        OpError::Empty => NavError::Empty,
        OpError::StaleSnapshot => NavError::StaleSnapshot,
        OpError::Validation(err) => NavError::Validation(err),
        OpError::MissingNode(id) | OpError::NotLeaf(id) | OpError::NotSplit(id) => {
            NavError::MissingSnapshotRect(id)
        }
        other => NavError::Validation(ValidationError::InvalidSelection(match other {
            OpError::NoParent(id) => id,
            OpError::MissingNode(id) => id,
            OpError::NotLeaf(id) => id,
            OpError::NotSplit(id) => id,
            _ => 0,
        })),
    }
}
