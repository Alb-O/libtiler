use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    error::OpError,
    geom::{Axis, Direction},
    ids::NodeId,
    limits::Summary,
    snapshot::Snapshot,
    tree::Tree,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResizeStrategy {
    Local,
    AncestorChain,
    DistributedBySlack,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EligibleSplit {
    pub split: NodeId,
    pub total: u32,
    pub current_a: u32,
    pub lo: u32,
    pub hi: u32,
}

impl EligibleSplit {
    pub(crate) fn slack(self, sign: i8) -> u32 {
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

pub(crate) fn resize_sign(dir: Direction, outward: bool) -> i8 {
    match (dir, outward) {
        (Direction::Right | Direction::Down, true) | (Direction::Left | Direction::Up, false) => 1,
        (Direction::Left | Direction::Up, true) | (Direction::Right | Direction::Down, false) => -1,
    }
}

pub(crate) fn distribute_resize(
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

pub(crate) fn eligible_splits<T>(
    tree: &Tree<T>,
    focus: NodeId,
    dir: Direction,
    snap: &Snapshot,
    summaries: &HashMap<NodeId, Summary>,
) -> Result<Vec<EligibleSplit>, OpError> {
    let focus_rect = snap.rect(focus).ok_or(OpError::MissingNode(focus))?;
    let mut out = Vec::new();
    for split_id in tree.ancestors_nearest_first(focus) {
        let split = tree.split(split_id).ok_or(OpError::NotSplit(split_id))?;
        let a_rect = snap.rect(split.a).ok_or(OpError::MissingNode(split.a))?;
        let b_rect = snap.rect(split.b).ok_or(OpError::MissingNode(split.b))?;
        let focus_in_a = tree.contains_in_subtree(split.a, focus);
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
        out.push(EligibleSplit {
            split: split_id,
            total,
            current_a: a_rect.extent(split.axis),
            lo: min_a.max(max_b.map_or(0, |max_b| total.saturating_sub(max_b))),
            hi: total.saturating_sub(min_b).min(max_a.unwrap_or(total)),
        });
    }
    Ok(out)
}
