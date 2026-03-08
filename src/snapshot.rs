use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    geom::{Axis, Rect},
    ids::{NodeId, Revision},
    limits::WeightPair,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScoreTuple {
    pub shortage_penalty: u128,
    pub overflow_penalty: u128,
    pub preference_penalty: u128,
    pub tie_break: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitTrace {
    pub split: NodeId,
    pub axis: Axis,
    pub total: u32,
    pub chosen_a: u32,
    pub score: ScoreTuple,
    pub weights: WeightPair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationKind {
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub node: NodeId,
    pub kind: ViolationKind,
    pub required: u32,
    pub actual: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub revision: Revision,
    pub root: Rect,
    pub node_rects: HashMap<NodeId, Rect>,
    pub split_traces: Vec<SplitTrace>,
    pub violations: Vec<Violation>,
    pub strict_feasible: bool,
}

impl Snapshot {
    #[must_use]
    pub fn rect(&self, node: NodeId) -> Option<Rect> {
        self.node_rects.get(&node).copied()
    }
}
