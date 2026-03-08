#![allow(dead_code)]

use std::collections::HashMap;

use libtiler::{
    Axis, BalancedPreset, Direction, LeafMeta, Node, NodeId, PairSpec, PresetKind, RebalanceMode,
    Rect, ResizeStrategy, ScoreTuple, Session, ShortageMode, SizeLimits, Slot, SolverPolicy,
    Summary, TallPreset, TieBreakMode, Tree, WidePreset, score, summarize,
};

pub fn root_rect(w: u32, h: u32) -> Rect {
    Rect { x: 0, y: 0, w, h }
}

pub fn meta(
    min_w: u32,
    min_h: u32,
    max_w: Option<u32>,
    max_h: Option<u32>,
    shrink: u16,
) -> LeafMeta {
    LeafMeta {
        limits: SizeLimits {
            min_w,
            min_h,
            max_w,
            max_h,
        },
        priority: libtiler::Priority { shrink, grow: 1 },
    }
}

pub fn choose_extent_oracle(spec: PairSpec, policy: &SolverPolicy) -> (u32, ScoreTuple) {
    (0..=spec.total)
        .map(|a| (a, score(spec, a, policy)))
        .min_by_key(|(a, score)| (*score, *a))
        .expect("oracle search space is never empty")
}

pub fn leaf_ids<T>(session: &Session<T>) -> Vec<NodeId> {
    session
        .tree
        .root
        .map(|root| session.tree.leaf_ids_dfs(root))
        .unwrap_or_default()
}

pub fn split_ids<T>(session: &Session<T>) -> Vec<NodeId> {
    session
        .tree
        .nodes
        .iter()
        .filter_map(|(id, node)| matches!(node, Node::Split(_)).then_some(*id))
        .collect()
}

pub fn assert_partition<T>(session: &Session<T>, root: Rect, snap: &libtiler::Snapshot) {
    let Some(tree_root) = session.tree.root else {
        assert!(snap.node_rects.is_empty());
        return;
    };
    let mut cells =
        vec![None; usize::try_from(root.w.saturating_mul(root.h)).expect("root too large")];
    for leaf in session.tree.leaf_ids_dfs(tree_root) {
        let rect = snap.rect(leaf).expect("missing leaf rect");
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let x_off = u32::try_from(x - root.left()).expect("x below root");
                let y_off = u32::try_from(y - root.top()).expect("y below root");
                let idx = usize::try_from(y_off * root.w + x_off).expect("cell index overflow");
                assert!(cells[idx].is_none(), "cell overlapped at ({x}, {y})");
                cells[idx] = Some(leaf);
            }
        }
    }
    assert!(
        cells.iter().all(Option::is_some),
        "partition left holes inside root"
    );
}

pub fn solve_reference<T>(
    tree: &Tree<T>,
    revision: u64,
    root: Rect,
    policy: &SolverPolicy,
) -> libtiler::Snapshot {
    let mut summaries = HashMap::new();
    let mut snapshot = libtiler::Snapshot {
        revision,
        root,
        node_rects: HashMap::new(),
        split_traces: Vec::new(),
        violations: Vec::new(),
        strict_feasible: true,
    };
    let Some(root_id) = tree.root else {
        return snapshot;
    };
    summarize(tree, root_id, &mut summaries).expect("tree should summarize");
    solve_reference_node(tree, root_id, root, &summaries, policy, &mut snapshot);
    snapshot.strict_feasible = snapshot.violations.is_empty();
    snapshot
}

fn solve_reference_node<T>(
    tree: &Tree<T>,
    id: NodeId,
    rect: Rect,
    summaries: &HashMap<NodeId, Summary>,
    policy: &SolverPolicy,
    out: &mut libtiler::Snapshot,
) {
    out.node_rects.insert(id, rect);
    match tree.nodes.get(&id).expect("missing node") {
        Node::Leaf(leaf) => record_leaf_violations(id, rect, &leaf.meta.limits, out),
        Node::Split(split) => {
            let sum_a = summaries[&split.a];
            let sum_b = summaries[&split.b];
            let spec = PairSpec {
                total: rect.extent(split.axis),
                min_a: sum_a.axis_limits(split.axis).0,
                min_b: sum_b.axis_limits(split.axis).0,
                max_a: sum_a.axis_limits(split.axis).1,
                max_b: sum_b.axis_limits(split.axis).1,
                wa: split.weights.a,
                wb: split.weights.b,
                sa: sum_a.shrink_cost,
                sb: sum_b.shrink_cost,
            };
            let (chosen_a, score) = choose_extent_oracle(spec, policy);
            let (rect_a, rect_b) = rect.split(split.axis, chosen_a);
            out.split_traces.push(libtiler::SplitTrace {
                split: id,
                axis: split.axis,
                total: spec.total,
                chosen_a,
                score,
                weights: split.weights,
            });
            solve_reference_node(tree, split.a, rect_a, summaries, policy, out);
            solve_reference_node(tree, split.b, rect_b, summaries, policy, out);
        }
    }
}

fn record_leaf_violations(
    node: NodeId,
    rect: Rect,
    limits: &SizeLimits,
    out: &mut libtiler::Snapshot,
) {
    if rect.w < limits.min_w {
        out.violations.push(libtiler::Violation {
            node,
            kind: libtiler::ViolationKind::MinWidth,
            required: limits.min_w,
            actual: rect.w,
        });
    }
    if rect.h < limits.min_h {
        out.violations.push(libtiler::Violation {
            node,
            kind: libtiler::ViolationKind::MinHeight,
            required: limits.min_h,
            actual: rect.h,
        });
    }
    if let Some(max_w) = limits.max_w.filter(|max_w| rect.w > *max_w) {
        out.violations.push(libtiler::Violation {
            node,
            kind: libtiler::ViolationKind::MaxWidth,
            required: max_w,
            actual: rect.w,
        });
    }
    if let Some(max_h) = limits.max_h.filter(|max_h| rect.h > *max_h) {
        out.violations.push(libtiler::Violation {
            node,
            kind: libtiler::ViolationKind::MaxHeight,
            required: max_h,
            actual: rect.h,
        });
    }
}

pub fn exercise_trace(bytes: &[u8]) -> Session<u16> {
    let mut session = Session::new();
    let _ = session
        .insert_root(0, LeafMeta::default())
        .expect("root insert should work");
    let mut next_payload = 1_u16;
    for byte in bytes {
        let leaves = leaf_ids(&session);
        let splits = split_ids(&session);
        let nodes = session.tree.nodes.keys().copied().collect::<Vec<_>>();
        match byte % 12 {
            0 => {
                let _ = session.split_focus(
                    axis(byte),
                    slot(byte),
                    next_payload,
                    LeafMeta::default(),
                    None,
                );
                next_payload += 1;
            }
            1 => {
                let _ = session.wrap_selection(
                    axis(byte),
                    slot(byte),
                    next_payload,
                    LeafMeta::default(),
                    None,
                );
                next_payload += 1;
            }
            2 if leaves.len() > 1 => {
                let leaf = leaves[usize::from(*byte) % leaves.len()];
                session.focus = Some(leaf);
                session.selection = Some(leaf);
                let _ = session.remove_focus();
            }
            3 if !splits.is_empty() => {
                let split = splits[usize::from(*byte) % splits.len()];
                session.focus = session.tree.first_leaf(split);
                session.selection = Some(split);
                let _ = session.toggle_axis();
            }
            4 if !splits.is_empty() => {
                let split = splits[usize::from(*byte) % splits.len()];
                session.focus = session.tree.first_leaf(split);
                session.selection = Some(split);
                let _ = session.mirror_selection(axis(byte));
            }
            5 if !splits.is_empty() => {
                let split = splits[usize::from(*byte) % splits.len()];
                session.focus = session.tree.first_leaf(split);
                session.selection = Some(split);
                let mode = if byte & 1 == 0 {
                    RebalanceMode::BinaryEqual
                } else {
                    RebalanceMode::LeafCount
                };
                let _ = session.rebalance_selection(mode);
            }
            6 if !splits.is_empty() => {
                let split = splits[usize::from(*byte) % splits.len()];
                session.focus = session.tree.first_leaf(split);
                session.selection = Some(split);
                let preset = match byte & 0b11 {
                    0 => PresetKind::Balanced(BalancedPreset {
                        start_axis: axis(byte),
                        alternate: byte & 0b100 != 0,
                    }),
                    1 => PresetKind::Tall(TallPreset {
                        master_slot: slot(byte),
                        root_weights: libtiler::WeightPair { a: 2, b: 1 },
                    }),
                    2 => PresetKind::Wide(WidePreset {
                        master_slot: slot(byte),
                        root_weights: libtiler::WeightPair { a: 1, b: 3 },
                    }),
                    _ => PresetKind::Dwindle(libtiler::DwindlePreset {
                        start_axis: axis(byte),
                        new_leaf_slot: slot(byte),
                    }),
                };
                let _ = session.apply_preset(preset);
            }
            7 if leaves.len() > 1 => {
                let focus = leaves[usize::from(*byte) % leaves.len()];
                session.focus = Some(focus);
                session.selection = Some(focus);
                let snap = session.solve(root_rect(9, 7), &SolverPolicy::default());
                let _ = session.focus_dir(direction(byte), &snap);
            }
            8 if leaves.len() > 1 => {
                let focus = leaves[usize::from(*byte) % leaves.len()];
                session.focus = Some(focus);
                session.selection = Some(focus);
                let snap = session.solve(root_rect(11, 9), &SolverPolicy::default());
                let _ = session.grow_focus(
                    direction(byte),
                    2 + u32::from(*byte & 0b11),
                    strategy(byte),
                    &snap,
                );
            }
            9 if leaves.len() > 1 => {
                let focus = leaves[usize::from(*byte) % leaves.len()];
                session.focus = Some(focus);
                session.selection = Some(focus);
                let snap = session.solve(root_rect(11, 9), &SolverPolicy::default());
                let _ = session.shrink_focus(
                    direction(byte),
                    1 + u32::from(*byte & 0b11),
                    strategy(byte),
                    &snap,
                );
            }
            10 if nodes.len() > 1 => {
                if let Some((a, b)) = first_disjoint_pair(&session, &nodes) {
                    let _ = session.swap_nodes(a, b);
                }
            }
            11 if nodes.len() > 1 && !splits.is_empty() => {
                let split = splits[usize::from(*byte) % splits.len()];
                let targets = nodes
                    .iter()
                    .copied()
                    .filter(|target| *target != split)
                    .collect::<Vec<_>>();
                if let Some(target) = targets.get(usize::from(*byte) % targets.len()) {
                    session.focus = session.tree.first_leaf(split);
                    session.selection = Some(split);
                    let _ = session.move_selection_as_sibling_of(*target, axis(byte), slot(byte));
                }
            }
            _ => {}
        }
        session
            .validate()
            .expect("trace should preserve session invariants");
    }
    session
}

fn first_disjoint_pair<T>(session: &Session<T>, nodes: &[NodeId]) -> Option<(NodeId, NodeId)> {
    for (idx, a) in nodes.iter().enumerate() {
        for b in nodes.iter().skip(idx + 1) {
            if !session.tree.contains_in_subtree(*a, *b)
                && !session.tree.contains_in_subtree(*b, *a)
            {
                return Some((*a, *b));
            }
        }
    }
    None
}

pub fn scale_session(session: &Session<u16>, factor: u32) -> Session<u16> {
    let mut scaled = session.clone();
    for node in scaled.tree.nodes.values_mut() {
        if let Node::Leaf(leaf) = node {
            leaf.meta.limits.min_w *= factor;
            leaf.meta.limits.min_h *= factor;
            leaf.meta.limits.max_w = leaf.meta.limits.max_w.map(|value| value * factor);
            leaf.meta.limits.max_h = leaf.meta.limits.max_h.map(|value| value * factor);
        }
    }
    scaled
}

pub fn axis(byte: &u8) -> Axis {
    if byte & 1 == 0 { Axis::X } else { Axis::Y }
}

pub fn slot(byte: &u8) -> Slot {
    if byte & 2 == 0 { Slot::A } else { Slot::B }
}

pub fn direction(byte: &u8) -> Direction {
    match byte & 0b11 {
        0 => Direction::Left,
        1 => Direction::Right,
        2 => Direction::Up,
        _ => Direction::Down,
    }
}

pub fn strategy(byte: &u8) -> ResizeStrategy {
    match byte % 3 {
        0 => ResizeStrategy::Local,
        1 => ResizeStrategy::AncestorChain,
        _ => ResizeStrategy::DistributedBySlack,
    }
}

pub fn stressed_policy(seed: u8) -> SolverPolicy {
    SolverPolicy {
        shortage_mode: if seed & 1 == 0 {
            ShortageMode::Equal
        } else {
            ShortageMode::ByShrinkPriority
        },
        overflow_mode: libtiler::OverflowMode::Uniform,
        tie_break: if seed & 2 == 0 {
            TieBreakMode::PreferA
        } else {
            TieBreakMode::PreferB
        },
    }
}
