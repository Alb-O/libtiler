use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    error::{NavError, OpError, SolveError, ValidationError},
    geom::{Axis, Direction, Rect, Slot},
    ids::{NodeId, Revision},
    limits::{LeafMeta, WeightPair, canonicalize_weights},
    nav::best_neighbor,
    preset::{PresetKind, build_preset_subtree, subtree_matches_preset},
    resize::{ResizeStrategy, distribute_resize, eligible_splits, resize_sign},
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
    tree: Tree<T>,
    focus: Option<NodeId>,
    selection: Option<NodeId>,
    revision: Revision,
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
        match self.tree.root_id() {
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
        self.try_solve(root, policy)
            .expect("session should maintain a valid and representable tree")
    }

    pub fn try_solve(&self, root: Rect, policy: &SolverPolicy) -> Result<Snapshot, SolveError> {
        crate::solver::solve_with_revision(&self.tree, root, self.revision, policy)
    }

    #[must_use]
    pub fn tree(&self) -> &Tree<T> {
        &self.tree
    }

    #[must_use]
    pub fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    #[must_use]
    pub fn selection(&self) -> Option<NodeId> {
        self.selection
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn set_focus_leaf(&mut self, id: NodeId) -> Result<(), OpError> {
        let old_focus = self.focus;
        let old_selection = self.selection;
        self.require_leaf(id)?;
        self.focus = Some(id);
        self.repair_selection_for_current_focus();
        self.validate()
            .map_err(OpError::Validation)
            .inspect_err(|_| {
                self.focus = old_focus;
                self.selection = old_selection;
            })
    }

    pub fn set_selection(&mut self, id: NodeId) -> Result<(), OpError> {
        let old_focus = self.focus;
        let old_selection = self.selection;
        self.require_node(id)?;
        if self.tree.is_leaf(id) {
            self.focus = Some(id);
            self.selection = Some(id);
        } else {
            let focus = self.require_focus_leaf()?;
            if !self.tree.contains_in_subtree(id, focus) {
                return Err(OpError::Validation(ValidationError::InvalidSelection(id)));
            }
            self.selection = Some(id);
        }
        self.validate()
            .map_err(OpError::Validation)
            .inspect_err(|_| {
                self.focus = old_focus;
                self.selection = old_selection;
            })
    }

    pub fn insert_root(&mut self, payload: T, meta: LeafMeta) -> Result<NodeId, OpError> {
        if self.tree.root_id().is_some() {
            return Err(OpError::NonEmpty);
        }
        let id = self.tree.new_leaf(payload, meta);
        self.tree.set_root(Some(id));
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
        let focus = self.require_focus_leaf()?;
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
        let focus = self.require_focus_leaf()?;
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
        let focus = self.require_focus_leaf()?;
        let old_selection = self.selection;
        let fallback = if self.tree.root_id() == Some(focus) {
            self.tree.set_root(None);
            None
        } else {
            let sibling = self
                .tree
                .sibling_of(focus)
                .ok_or(OpError::NoParent(focus))?;
            let replacement = self.tree.collapse_unary_parent(focus).unwrap_or(sibling);
            Some(replacement)
        };
        self.tree.remove_node(focus);
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
        let focus = self.require_focus_leaf()?;
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
                    self.tree.set_root(Some(b));
                    self.tree.set_parent(b, None);
                }
            }
            match parent_b {
                Some(parent) => self.tree.replace_child(parent, b, a),
                None => {
                    self.tree.set_root(Some(a));
                    self.tree.set_parent(a, None);
                }
            }
            self.tree.set_parent(a, parent_b);
            self.tree.set_parent(b, parent_a);
        }
        self.repair_after_mutation(focus, old_selection, self.tree.root_id());
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
        let focus = self.require_focus_leaf()?;
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
        if subtree_matches_preset(&self.tree, selection, preset)? {
            return Ok(());
        }
        let focus = self.require_focus_leaf()?;
        let old_selection = self.selection;
        let parent = self.tree.parent_of(selection);
        let leaves = self.tree.leaf_ids_dfs(selection);
        let split_ids = collect_split_ids(&self.tree, selection);
        for leaf in &leaves {
            self.tree.set_parent(*leaf, None);
        }
        for split in split_ids {
            self.tree.remove_node(split);
        }
        let rebuilt = build_preset_subtree(&mut self.tree, &leaves, preset)?;
        match parent {
            Some(parent) => {
                self.tree.replace_child(parent, selection, rebuilt);
                self.tree.set_parent(rebuilt, Some(parent));
            }
            None => {
                self.tree.set_root(Some(rebuilt));
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
        let focus = self.require_focus_leaf()?;
        let mut summaries = HashMap::new();
        if let Some(root) = self.tree.root_id() {
            summarize(&self.tree, root, &mut summaries).map_err(OpError::Validation)?;
        }
        let eligible = eligible_splits(&self.tree, focus, dir, snap, &summaries)?;
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
            .root_id()
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
                self.tree.set_root(Some(split_id));
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
        self.focus = if self.tree.root_id().is_none() {
            None
        } else if self.tree.is_leaf(old_focus) {
            Some(old_focus)
        } else {
            replacement_site.and_then(|id| self.tree.first_leaf(id))
        };

        self.selection = match (self.tree.root_id(), self.focus) {
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

    fn require_focus_leaf(&self) -> Result<NodeId, OpError> {
        let focus = self.focus.ok_or(OpError::Empty)?;
        if self.tree.is_leaf(focus) {
            Ok(focus)
        } else {
            Err(OpError::NotLeaf(focus))
        }
    }

    fn split_mut(&mut self, id: NodeId) -> Result<&mut SplitNode, OpError> {
        self.tree.split_mut(id).ok_or(OpError::NotSplit(id))
    }

    fn require_node(&self, id: NodeId) -> Result<(), OpError> {
        if self.tree.contains(id) {
            Ok(())
        } else {
            Err(OpError::MissingNode(id))
        }
    }

    fn require_leaf(&self, id: NodeId) -> Result<(), OpError> {
        self.require_node(id)?;
        if self.tree.is_leaf(id) {
            Ok(())
        } else {
            Err(OpError::NotLeaf(id))
        }
    }

    fn bump_revision(&mut self) {
        self.revision += 1;
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
    match tree.node(current)? {
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
