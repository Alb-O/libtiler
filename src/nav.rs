use std::collections::HashMap;

use crate::{
    geom::{Direction, Rect, orth_gap},
    ids::NodeId,
    tree::Tree,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NavScore {
    pub primary_gap: u32,
    pub orth_gap: u32,
    pub orth_center_delta: u64,
    pub tree_order_rank: usize,
}

#[must_use]
pub fn best_neighbor<T>(
    tree: &Tree<T>,
    leaf_rects: &HashMap<NodeId, Rect>,
    current: NodeId,
    dir: Direction,
) -> Option<NodeId> {
    let current_rect = leaf_rects.get(&current).copied()?;
    let order = tree
        .root_id()
        .map(|root| tree.leaf_ids_dfs(root))
        .unwrap_or_default();
    let order_rank = order
        .into_iter()
        .enumerate()
        .map(|(idx, id)| (id, idx))
        .collect::<HashMap<_, _>>();

    leaf_rects
        .iter()
        .filter(|(id, _)| **id != current)
        .filter_map(|(id, rect)| {
            nav_score(
                current_rect,
                *rect,
                dir,
                *order_rank.get(id).unwrap_or(&usize::MAX),
            )
            .map(|score| (*id, score))
        })
        .min_by_key(|(_, score)| *score)
        .map(|(id, _)| id)
}

#[must_use]
pub fn nav_score(current: Rect, candidate: Rect, dir: Direction, rank: usize) -> Option<NavScore> {
    let (eligible, primary_gap, orth_gap_value, orth_center_delta) = match dir {
        Direction::Left => {
            let eligible = candidate.right() <= current.left();
            let primary_gap = u32::try_from(current.left() - candidate.right()).ok()?;
            let orth_gap_value = orth_gap(
                current.top(),
                current.bottom(),
                candidate.top(),
                candidate.bottom(),
            );
            let orth_center_delta = current
                .center_twice_orth(crate::geom::Axis::X)
                .abs_diff(candidate.center_twice_orth(crate::geom::Axis::X));
            (eligible, primary_gap, orth_gap_value, orth_center_delta)
        }
        Direction::Right => {
            let eligible = candidate.left() >= current.right();
            let primary_gap = u32::try_from(candidate.left() - current.right()).ok()?;
            let orth_gap_value = orth_gap(
                current.top(),
                current.bottom(),
                candidate.top(),
                candidate.bottom(),
            );
            let orth_center_delta = current
                .center_twice_orth(crate::geom::Axis::X)
                .abs_diff(candidate.center_twice_orth(crate::geom::Axis::X));
            (eligible, primary_gap, orth_gap_value, orth_center_delta)
        }
        Direction::Up => {
            let eligible = candidate.bottom() <= current.top();
            let primary_gap = u32::try_from(current.top() - candidate.bottom()).ok()?;
            let orth_gap_value = orth_gap(
                current.left(),
                current.right(),
                candidate.left(),
                candidate.right(),
            );
            let orth_center_delta = current
                .center_twice_orth(crate::geom::Axis::Y)
                .abs_diff(candidate.center_twice_orth(crate::geom::Axis::Y));
            (eligible, primary_gap, orth_gap_value, orth_center_delta)
        }
        Direction::Down => {
            let eligible = candidate.top() >= current.bottom();
            let primary_gap = u32::try_from(candidate.top() - current.bottom()).ok()?;
            let orth_gap_value = orth_gap(
                current.left(),
                current.right(),
                candidate.left(),
                candidate.right(),
            );
            let orth_center_delta = current
                .center_twice_orth(crate::geom::Axis::Y)
                .abs_diff(candidate.center_twice_orth(crate::geom::Axis::Y));
            (eligible, primary_gap, orth_gap_value, orth_center_delta)
        }
    };
    eligible.then_some(NavScore {
        primary_gap,
        orth_gap: orth_gap_value,
        orth_center_delta,
        tree_order_rank: rank,
    })
}
