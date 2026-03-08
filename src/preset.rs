use serde::{Deserialize, Serialize};

use crate::{
    geom::{Axis, Slot},
    limits::WeightPair,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancedPreset {
    pub start_axis: Axis,
    pub alternate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DwindlePreset {
    pub start_axis: Axis,
    pub new_leaf_slot: Slot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TallPreset {
    pub master_slot: Slot,
    pub root_weights: WeightPair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidePreset {
    pub master_slot: Slot,
    pub root_weights: WeightPair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetKind {
    Balanced(BalancedPreset),
    Dwindle(DwindlePreset),
    Tall(TallPreset),
    Wide(WidePreset),
}
