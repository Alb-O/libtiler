use serde::{Deserialize, Serialize};

use crate::{
    error::OpError,
    geom::{Axis, Slot},
    ids::NodeId,
    limits::{WeightPair, canonicalize_weights},
    tree::Tree,
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

pub(crate) fn build_preset_subtree<T>(
    tree: &mut Tree<T>,
    leaves: &[NodeId],
    preset: PresetKind,
) -> Result<NodeId, OpError> {
    match preset {
        PresetKind::Balanced(preset) => build_balanced(tree, leaves, preset),
        PresetKind::Dwindle(preset) => {
            build_dwindle(tree, leaves, preset.start_axis, preset.new_leaf_slot)
        }
        PresetKind::Tall(preset) => build_tall(tree, leaves, preset),
        PresetKind::Wide(preset) => build_wide(tree, leaves, preset),
    }
}

pub(crate) fn subtree_matches_preset<T>(
    tree: &Tree<T>,
    root: NodeId,
    preset: PresetKind,
) -> Result<bool, OpError> {
    let leaves = tree.leaf_ids_dfs(root);
    match preset {
        PresetKind::Balanced(preset) => Ok(matches_balanced(tree, root, &leaves, preset)),
        PresetKind::Dwindle(preset) => Ok(matches_dwindle(
            tree,
            root,
            &leaves,
            preset.start_axis,
            preset.new_leaf_slot,
        )),
        PresetKind::Tall(preset) => Ok(matches_tall(tree, root, &leaves, preset)),
        PresetKind::Wide(preset) => Ok(matches_wide(tree, root, &leaves, preset)),
    }
}

fn build_balanced<T>(
    tree: &mut Tree<T>,
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
    let a = build_balanced(
        tree,
        &leaves[..mid],
        BalancedPreset {
            start_axis: next_axis,
            alternate: preset.alternate,
        },
    )?;
    let b = build_balanced(
        tree,
        &leaves[mid..],
        BalancedPreset {
            start_axis: next_axis,
            alternate: preset.alternate,
        },
    )?;
    Ok(new_internal_split(
        tree,
        preset.start_axis,
        a,
        b,
        canonicalize_weights(mid as u32, (leaves.len() - mid) as u32),
    ))
}

fn build_dwindle<T>(
    tree: &mut Tree<T>,
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
    let rest = build_dwindle(tree, &leaves[1..], toggle_axis(axis), slot)?;
    let (a, b) = match slot {
        Slot::A => (rest, first),
        Slot::B => (first, rest),
    };
    Ok(new_internal_split(tree, axis, a, b, WeightPair::default()))
}

fn build_tall<T>(
    tree: &mut Tree<T>,
    leaves: &[NodeId],
    preset: TallPreset,
) -> Result<NodeId, OpError> {
    if leaves.is_empty() {
        return Err(OpError::Empty);
    }
    if leaves.len() == 1 {
        return Ok(leaves[0]);
    }
    let master = leaves[0];
    let stack = build_equal_linear(tree, &leaves[1..], Axis::Y)?;
    let (a, b) = match preset.master_slot {
        Slot::A => (master, stack),
        Slot::B => (stack, master),
    };
    Ok(new_internal_split(
        tree,
        Axis::X,
        a,
        b,
        checked_weights(preset.root_weights)?,
    ))
}

fn build_wide<T>(
    tree: &mut Tree<T>,
    leaves: &[NodeId],
    preset: WidePreset,
) -> Result<NodeId, OpError> {
    if leaves.is_empty() {
        return Err(OpError::Empty);
    }
    if leaves.len() == 1 {
        return Ok(leaves[0]);
    }
    let master = leaves[0];
    let stack = build_equal_linear(tree, &leaves[1..], Axis::X)?;
    let (a, b) = match preset.master_slot {
        Slot::A => (master, stack),
        Slot::B => (stack, master),
    };
    Ok(new_internal_split(
        tree,
        Axis::Y,
        a,
        b,
        checked_weights(preset.root_weights)?,
    ))
}

fn build_equal_linear<T>(
    tree: &mut Tree<T>,
    leaves: &[NodeId],
    axis: Axis,
) -> Result<NodeId, OpError> {
    if leaves.is_empty() {
        return Err(OpError::Empty);
    }
    if leaves.len() == 1 {
        return Ok(leaves[0]);
    }
    let head = leaves[0];
    let rest = build_equal_linear(tree, &leaves[1..], axis)?;
    Ok(new_internal_split(
        tree,
        axis,
        head,
        rest,
        canonicalize_weights(1, (leaves.len() - 1) as u32),
    ))
}

fn matches_balanced<T>(
    tree: &Tree<T>,
    id: NodeId,
    leaves: &[NodeId],
    preset: BalancedPreset,
) -> bool {
    if leaves.is_empty() {
        return false;
    }
    if leaves.len() == 1 {
        return tree.is_leaf(id) && id == leaves[0];
    }
    let Some(split) = tree.split(id) else {
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
    matches_balanced(tree, split.a, &leaves[..mid], next)
        && matches_balanced(tree, split.b, &leaves[mid..], next)
}

fn matches_dwindle<T>(
    tree: &Tree<T>,
    id: NodeId,
    leaves: &[NodeId],
    axis: Axis,
    slot: Slot,
) -> bool {
    if leaves.is_empty() {
        return false;
    }
    if leaves.len() == 1 {
        return tree.is_leaf(id) && id == leaves[0];
    }
    let Some(split) = tree.split(id) else {
        return false;
    };
    if split.axis != axis || split.weights != WeightPair::default() {
        return false;
    }
    match slot {
        Slot::A => {
            split.b == leaves[0]
                && tree.is_leaf(split.b)
                && matches_dwindle(tree, split.a, &leaves[1..], toggle_axis(axis), slot)
        }
        Slot::B => {
            split.a == leaves[0]
                && tree.is_leaf(split.a)
                && matches_dwindle(tree, split.b, &leaves[1..], toggle_axis(axis), slot)
        }
    }
}

fn matches_tall<T>(tree: &Tree<T>, id: NodeId, leaves: &[NodeId], preset: TallPreset) -> bool {
    if leaves.is_empty() {
        return false;
    }
    if leaves.len() == 1 {
        return tree.is_leaf(id) && id == leaves[0];
    }
    let Some(split) = tree.split(id) else {
        return false;
    };
    if split.axis != Axis::X || split.weights != preset.root_weights {
        return false;
    }
    match preset.master_slot {
        Slot::A => {
            split.a == leaves[0]
                && tree.is_leaf(split.a)
                && matches_equal_linear(tree, split.b, &leaves[1..], Axis::Y)
        }
        Slot::B => {
            split.b == leaves[0]
                && tree.is_leaf(split.b)
                && matches_equal_linear(tree, split.a, &leaves[1..], Axis::Y)
        }
    }
}

fn matches_wide<T>(tree: &Tree<T>, id: NodeId, leaves: &[NodeId], preset: WidePreset) -> bool {
    if leaves.is_empty() {
        return false;
    }
    if leaves.len() == 1 {
        return tree.is_leaf(id) && id == leaves[0];
    }
    let Some(split) = tree.split(id) else {
        return false;
    };
    if split.axis != Axis::Y || split.weights != preset.root_weights {
        return false;
    }
    match preset.master_slot {
        Slot::A => {
            split.a == leaves[0]
                && tree.is_leaf(split.a)
                && matches_equal_linear(tree, split.b, &leaves[1..], Axis::X)
        }
        Slot::B => {
            split.b == leaves[0]
                && tree.is_leaf(split.b)
                && matches_equal_linear(tree, split.a, &leaves[1..], Axis::X)
        }
    }
}

fn matches_equal_linear<T>(tree: &Tree<T>, id: NodeId, leaves: &[NodeId], axis: Axis) -> bool {
    if leaves.is_empty() {
        return false;
    }
    if leaves.len() == 1 {
        return tree.is_leaf(id) && id == leaves[0];
    }
    let Some(split) = tree.split(id) else {
        return false;
    };
    split.axis == axis
        && split.weights == canonicalize_weights(1, (leaves.len() - 1) as u32)
        && split.a == leaves[0]
        && tree.is_leaf(split.a)
        && matches_equal_linear(tree, split.b, &leaves[1..], axis)
}

fn new_internal_split<T>(
    tree: &mut Tree<T>,
    axis: Axis,
    a: NodeId,
    b: NodeId,
    weights: WeightPair,
) -> NodeId {
    let split_id = tree.new_split(axis, a, b, weights);
    tree.set_parent(a, Some(split_id));
    tree.set_parent(b, Some(split_id));
    split_id
}

fn toggle_axis(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

fn checked_weights(weights: WeightPair) -> Result<WeightPair, OpError> {
    if weights.a == 0 && weights.b == 0 {
        Err(OpError::InvalidWeights)
    } else {
        Ok(weights)
    }
}
