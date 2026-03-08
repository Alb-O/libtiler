use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    error::ValidationError,
    geom::{Axis, Slot},
    ids::NodeId,
    limits::{LeafMeta, WeightPair},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafNode<T> {
    pub parent: Option<NodeId>,
    pub payload: T,
    pub meta: LeafMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitNode {
    pub parent: Option<NodeId>,
    pub axis: Axis,
    pub a: NodeId,
    pub b: NodeId,
    pub weights: WeightPair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Node<T> {
    Leaf(LeafNode<T>),
    Split(SplitNode),
}

impl<T> Node<T> {
    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        match self {
            Self::Leaf(leaf) => leaf.parent,
            Self::Split(split) => split.parent,
        }
    }

    pub fn parent_mut(&mut self) -> &mut Option<NodeId> {
        match self {
            Self::Leaf(leaf) => &mut leaf.parent,
            Self::Split(split) => &mut split.parent,
        }
    }

    #[must_use]
    pub fn as_split(&self) -> Option<&SplitNode> {
        match self {
            Self::Split(split) => Some(split),
            Self::Leaf(_) => None,
        }
    }

    #[must_use]
    pub fn as_split_mut(&mut self) -> Option<&mut SplitNode> {
        match self {
            Self::Split(split) => Some(split),
            Self::Leaf(_) => None,
        }
    }

    #[must_use]
    pub fn as_leaf(&self) -> Option<&LeafNode<T>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Split(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree<T> {
    pub root: Option<NodeId>,
    pub nodes: HashMap<NodeId, Node<T>>,
    pub next_id: NodeId,
}

impl<T> Default for Tree<T> {
    fn default() -> Self {
        Self {
            root: None,
            nodes: HashMap::new(),
            next_id: 1,
        }
    }
}

impl<T> Tree<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match self.root {
            None => {
                if self.nodes.is_empty() {
                    Ok(())
                } else {
                    let extra = self.nodes.keys().copied().min().unwrap_or_default();
                    Err(ValidationError::Unreachable(extra))
                }
            }
            Some(root) => {
                let root_node = self
                    .nodes
                    .get(&root)
                    .ok_or(ValidationError::MissingRoot(root))?;
                if root_node.parent().is_some() {
                    return Err(ValidationError::RootHasParent(root));
                }
                let mut visited = HashSet::new();
                self.validate_node(root, None, &mut visited)?;
                if let Some(unreachable) =
                    self.nodes.keys().copied().find(|id| !visited.contains(id))
                {
                    return Err(ValidationError::Unreachable(unreachable));
                }
                Ok(())
            }
        }
    }

    fn validate_node(
        &self,
        id: NodeId,
        expected_parent: Option<NodeId>,
        visited: &mut HashSet<NodeId>,
    ) -> Result<(), ValidationError> {
        if !visited.insert(id) {
            return Err(ValidationError::Cycle(id));
        }
        let node = self
            .nodes
            .get(&id)
            .ok_or(ValidationError::MissingNode(id))?;
        if node.parent() != expected_parent {
            return Err(ValidationError::ParentMismatch {
                node: id,
                expected: expected_parent.unwrap_or_default(),
                actual: node.parent(),
            });
        }
        match node {
            Node::Leaf(leaf) => {
                let limits = leaf.meta.limits;
                if limits.max_w.is_some_and(|max_w| limits.min_w > max_w)
                    || limits.max_h.is_some_and(|max_h| limits.min_h > max_h)
                    || leaf.meta.priority.shrink == 0
                    || leaf.meta.priority.grow == 0
                {
                    return Err(ValidationError::InvalidLeafLimits(id));
                }
            }
            Node::Split(split) => {
                if split.a == split.b {
                    return Err(ValidationError::DuplicateChild {
                        split: id,
                        child: split.a,
                    });
                }
                if split.weights.a == 0 && split.weights.b == 0 {
                    return Err(ValidationError::InvalidWeights(id));
                }
                self.validate_child(id, split.a, visited)?;
                self.validate_child(id, split.b, visited)?;
            }
        }
        Ok(())
    }

    fn validate_child(
        &self,
        parent: NodeId,
        child: NodeId,
        visited: &mut HashSet<NodeId>,
    ) -> Result<(), ValidationError> {
        if !self.nodes.contains_key(&child) {
            return Err(ValidationError::MissingNode(child));
        }
        self.validate_node(child, Some(parent), visited)
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    #[must_use]
    pub fn is_leaf(&self, id: NodeId) -> bool {
        matches!(self.nodes.get(&id), Some(Node::Leaf(_)))
    }

    #[must_use]
    pub fn is_split(&self, id: NodeId) -> bool {
        matches!(self.nodes.get(&id), Some(Node::Split(_)))
    }

    #[must_use]
    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(Node::parent)
    }

    pub fn new_leaf(&mut self, payload: T, meta: LeafMeta) -> NodeId {
        let id = self.alloc_id();
        self.nodes.insert(
            id,
            Node::Leaf(LeafNode {
                parent: None,
                payload,
                meta,
            }),
        );
        id
    }

    pub fn new_split(&mut self, axis: Axis, a: NodeId, b: NodeId, weights: WeightPair) -> NodeId {
        let id = self.alloc_id();
        self.nodes.insert(
            id,
            Node::Split(SplitNode {
                parent: None,
                axis,
                a,
                b,
                weights,
            }),
        );
        id
    }

    pub fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) {
        *self
            .nodes
            .get_mut(&id)
            .expect("node missing when setting parent")
            .parent_mut() = parent;
    }

    pub fn replace_child(&mut self, parent: NodeId, old: NodeId, new: NodeId) {
        let split = self
            .nodes
            .get_mut(&parent)
            .and_then(Node::as_split_mut)
            .expect("parent missing or not split");
        if split.a == old {
            split.a = new;
        } else if split.b == old {
            split.b = new;
        } else {
            panic!("old child not found under parent");
        }
        self.set_parent(new, Some(parent));
    }

    pub fn replace_node_in_parent_or_root(&mut self, old: NodeId, new: NodeId) {
        match self.parent_of(old) {
            Some(parent) => self.replace_child(parent, old, new),
            None => {
                self.root = Some(new);
                self.set_parent(new, None);
            }
        }
    }

    pub fn children_of(&self, id: NodeId) -> Option<(NodeId, NodeId)> {
        self.nodes
            .get(&id)
            .and_then(Node::as_split)
            .map(|split| (split.a, split.b))
    }

    #[must_use]
    pub fn sibling_of(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent_of(id)?;
        let split = self.nodes.get(&parent)?.as_split()?;
        if split.a == id {
            Some(split.b)
        } else if split.b == id {
            Some(split.a)
        } else {
            None
        }
    }

    #[must_use]
    pub fn path_to_root(&self, mut id: NodeId) -> Vec<NodeId> {
        let mut out = vec![id];
        while let Some(parent) = self.parent_of(id) {
            out.push(parent);
            id = parent;
        }
        out
    }

    #[must_use]
    pub fn ancestors_nearest_first(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut cursor = self.parent_of(id);
        while let Some(parent) = cursor {
            out.push(parent);
            cursor = self.parent_of(parent);
        }
        out
    }

    #[must_use]
    pub fn contains_in_subtree(&self, root: NodeId, needle: NodeId) -> bool {
        if root == needle {
            return true;
        }
        match self.nodes.get(&root) {
            Some(Node::Leaf(_)) | None => false,
            Some(Node::Split(split)) => {
                self.contains_in_subtree(split.a, needle)
                    || self.contains_in_subtree(split.b, needle)
            }
        }
    }

    #[must_use]
    pub fn first_leaf(&self, id: NodeId) -> Option<NodeId> {
        match self.nodes.get(&id)? {
            Node::Leaf(_) => Some(id),
            Node::Split(split) => self
                .first_leaf(split.a)
                .or_else(|| self.first_leaf(split.b)),
        }
    }

    #[must_use]
    pub fn leaf_ids_dfs(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_leaf_ids(root, &mut out);
        out
    }

    fn collect_leaf_ids(&self, id: NodeId, out: &mut Vec<NodeId>) {
        match self
            .nodes
            .get(&id)
            .expect("missing node in collect_leaf_ids")
        {
            Node::Leaf(_) => out.push(id),
            Node::Split(split) => {
                self.collect_leaf_ids(split.a, out);
                self.collect_leaf_ids(split.b, out);
            }
        }
    }

    pub fn swap_parent_slots(&mut self, parent: NodeId) {
        let split = self
            .nodes
            .get_mut(&parent)
            .and_then(Node::as_split_mut)
            .expect("split missing");
        std::mem::swap(&mut split.a, &mut split.b);
    }

    pub fn collapse_unary_parent(&mut self, removed_child: NodeId) -> Option<NodeId> {
        let parent = self.parent_of(removed_child)?;
        let sibling = self.sibling_of(removed_child)?;
        let grand = self.parent_of(parent);
        if let Some(grand) = grand {
            self.replace_child(grand, parent, sibling);
        } else {
            self.root = Some(sibling);
            self.set_parent(sibling, None);
        }
        self.nodes.remove(&parent);
        Some(sibling)
    }

    pub fn remove_subtree_ids(&mut self, id: NodeId) -> Vec<NodeId> {
        let mut removed = Vec::new();
        self.collect_remove(id, &mut removed);
        removed
    }

    fn collect_remove(&mut self, id: NodeId, removed: &mut Vec<NodeId>) {
        if let Some(node) = self.nodes.remove(&id) {
            match node {
                Node::Leaf(_) => removed.push(id),
                Node::Split(split) => {
                    self.collect_remove(split.a, removed);
                    self.collect_remove(split.b, removed);
                    removed.push(id);
                }
            }
        }
    }

    pub fn detach_subtree(&mut self, id: NodeId) {
        if self.root == Some(id) {
            self.root = None;
            self.set_parent(id, None);
            return;
        }
        let parent = self.parent_of(id).expect("detached subtree missing parent");
        let sibling = self
            .sibling_of(id)
            .expect("detached subtree missing sibling");
        let grand = self.parent_of(parent);
        if let Some(grand) = grand {
            self.replace_child(grand, parent, sibling);
        } else {
            self.root = Some(sibling);
            self.set_parent(sibling, None);
        }
        self.nodes.remove(&parent);
        self.set_parent(id, None);
    }

    pub fn attach_as_sibling(
        &mut self,
        target: NodeId,
        incoming: NodeId,
        axis: Axis,
        slot: Slot,
        weights: WeightPair,
    ) -> NodeId {
        let (a, b) = match slot {
            Slot::A => (incoming, target),
            Slot::B => (target, incoming),
        };
        let parent_of_target = self.parent_of(target);
        let split_id = self.new_split(axis, a, b, weights);
        self.set_parent(a, Some(split_id));
        self.set_parent(b, Some(split_id));
        match parent_of_target {
            Some(parent) => {
                self.replace_child(parent, target, split_id);
                self.set_parent(split_id, Some(parent));
            }
            None => {
                self.root = Some(split_id);
                self.set_parent(split_id, None);
            }
        }
        split_id
    }

    fn alloc_id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}
