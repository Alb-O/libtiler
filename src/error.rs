use std::fmt::{Display, Formatter};

use crate::ids::NodeId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    MissingRoot(NodeId),
    RootHasParent(NodeId),
    MissingNode(NodeId),
    ParentMismatch {
        node: NodeId,
        expected: NodeId,
        actual: Option<NodeId>,
    },
    DuplicateChild {
        split: NodeId,
        child: NodeId,
    },
    Cycle(NodeId),
    Unreachable(NodeId),
    InvalidWeights(NodeId),
    InvalidLeafLimits(NodeId),
    NonLeafFocus(NodeId),
    InvalidSelection(NodeId),
    EmptyStateInconsistent,
}

impl Display for ValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
    Validation(ValidationError),
    Infeasible,
}

impl Display for SolveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for SolveError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavError {
    Empty,
    StaleSnapshot,
    MissingSnapshotRect(NodeId),
    NoCandidate,
    Validation(ValidationError),
}

impl Display for NavError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for NavError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    Empty,
    NonEmpty,
    MissingNode(NodeId),
    NotLeaf(NodeId),
    NotSplit(NodeId),
    NoParent(NodeId),
    StaleSnapshot,
    InvalidWeights,
    AncestorConflict,
    SameNode,
    TargetInsideSelection,
    Validation(ValidationError),
}

impl Display for OpError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for OpError {}
