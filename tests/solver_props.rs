mod common;

use std::collections::HashMap;

use common::{
    assert_partition, choose_extent_oracle, exercise_trace, root_rect, solve_reference,
    stressed_policy,
};
use libtiler::{
    Axis, LeafMeta, PairSpec, Session, Slot, SolverPolicy, choose_extent_with_score, solve_strict,
    summarize,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn allocator_matches_oracle(
        total in 0_u32..16,
        min_a in 0_u32..8,
        min_b in 0_u32..8,
        wa in 0_u32..6,
        wb in 0_u32..6,
        sa in 1_u64..8,
        sb in 1_u64..8,
        max_a_extra in 0_u32..8,
        max_b_extra in 0_u32..8,
        use_max_a in any::<bool>(),
        use_max_b in any::<bool>(),
        seed in any::<u8>(),
    ) {
        let wa = if wa == 0 && wb == 0 { 1 } else { wa };
        let max_a = use_max_a.then_some(min_a + max_a_extra);
        let max_b = use_max_b.then_some(min_b + max_b_extra);
        let spec = PairSpec {
            total,
            min_a,
            min_b,
            max_a,
            max_b,
            wa,
            wb,
            sa,
            sb,
        };
        let policy = stressed_policy(seed);
        let expected = choose_extent_oracle(spec, &policy);
        let actual = choose_extent_with_score(spec, &policy);
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn production_solver_matches_reference_and_partitions(bytes in prop::collection::vec(any::<u8>(), 1..48), w in 0_u32..9, h in 0_u32..9, seed in any::<u8>()) {
        let session = exercise_trace(&bytes);
        let root = root_rect(w, h);
        let policy = stressed_policy(seed);
        let production = session.solve(root, &policy);
        let reference = solve_reference(&session.tree, session.revision, root, &policy);
        prop_assert_eq!(&production.node_rects, &reference.node_rects);
        prop_assert_eq!(&production.violations, &reference.violations);
        prop_assert_eq!(production.strict_feasible, reference.strict_feasible);
        assert_partition(&session, root, &production);

        let encoded = serde_json::to_string(&session).expect("session should serialize");
        let decoded: Session<u16> = serde_json::from_str(&encoded).expect("session should deserialize");
        let replay = decoded.solve(root, &policy);
        prop_assert_eq!(production, replay);
    }
}

#[test]
fn summary_matches_bruteforce_feasibility_envelope() {
    let session = exercise_trace(&[0, 1, 7, 0, 3, 6, 5, 2, 8, 11, 4, 10]);
    let root = session.tree.root.expect("non-empty session");
    let mut summaries = HashMap::new();
    let summary = summarize(&session.tree, root, &mut summaries).expect("summary should build");
    let policy = SolverPolicy::default();

    let strict_pairs = (0_u32..=20)
        .flat_map(|w| (0_u32..=20).map(move |h| (w, h)))
        .filter(|(w, h)| solve_strict(&session.tree, root_rect(*w, *h), &policy).is_ok())
        .collect::<Vec<_>>();

    let min_w = strict_pairs
        .iter()
        .map(|(w, _)| *w)
        .min()
        .expect("some strict solution should exist");
    let min_h = strict_pairs
        .iter()
        .map(|(_, h)| *h)
        .min()
        .expect("some strict solution should exist");
    let max_w = strict_pairs.iter().map(|(w, _)| *w).max();
    let max_h = strict_pairs.iter().map(|(_, h)| *h).max();

    assert_eq!(summary.min_w, min_w);
    assert_eq!(summary.min_h, min_h);
    assert_eq!(
        summary.max_w.unwrap_or(20),
        max_w.expect("strict widths should exist")
    );
    assert_eq!(
        summary.max_h.unwrap_or(20),
        max_h.expect("strict heights should exist")
    );
}

#[test]
fn scale_symmetry_holds_with_exact_integer_arithmetic() {
    let mut session = Session::new();
    let _a = session
        .insert_root(1_u16, LeafMeta::default())
        .expect("insert root");
    let _b = session
        .split_focus(Axis::X, Slot::B, 2_u16, LeafMeta::default(), None)
        .expect("split x");
    let _c = session
        .split_focus(Axis::Y, Slot::B, 3_u16, LeafMeta::default(), None)
        .expect("split y");
    let mut scaled = session.clone();
    for node in scaled.tree.nodes.values_mut() {
        if let libtiler::Node::Leaf(leaf) = node {
            leaf.meta.limits.min_w *= 3;
            leaf.meta.limits.min_h *= 3;
        }
    }
    let policy = SolverPolicy::default();
    let base = session.solve(root_rect(12, 12), &policy);
    let scaled_snap = scaled.solve(root_rect(36, 36), &policy);

    for (id, rect) in &base.node_rects {
        let scaled_rect = scaled_snap.rect(*id).expect("scaled rect missing");
        assert_eq!(scaled_rect.x, rect.x * 3);
        assert_eq!(scaled_rect.y, rect.y * 3);
        assert_eq!(scaled_rect.w, rect.w * 3);
        assert_eq!(scaled_rect.h, rect.h * 3);
    }
}
