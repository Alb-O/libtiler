mod common;

use common::root_rect;
use libtiler::{
    Axis, Direction, LeafMeta, NavError, OpError, ResizeStrategy, Session, Slot, SolverPolicy,
    ValidationError,
};

fn two_leaf_session() -> Session<u8> {
    let mut session = Session::new();
    let _ = session
        .insert_root(1, LeafMeta::default())
        .expect("insert root");
    let _ = session
        .split_focus(Axis::X, Slot::B, 2, LeafMeta::default(), None)
        .expect("split root");
    session
}

#[test]
fn focus_and_selection_ops_do_not_stale_snapshots() {
    let mut session = two_leaf_session();
    let root = root_rect(12, 8);
    let snap = session.solve(root, &SolverPolicy::default());

    session
        .focus_dir(Direction::Right, &snap)
        .expect("focus move should accept current snapshot");
    session
        .select_parent()
        .expect("parent selection should work");
    session.select_focus();
    session
        .focus_dir(Direction::Left, &snap)
        .expect("selection-only ops should keep the original snapshot fresh");

    assert_eq!(session.revision(), snap.revision);
    assert_eq!(
        session
            .try_solve(root, &SolverPolicy::default())
            .expect("try_solve should remain fresh")
            .revision,
        session.revision()
    );
}

#[test]
fn structural_mutation_stales_old_snapshot() {
    let mut session = two_leaf_session();
    let root = root_rect(12, 8);
    let snap = session.solve(root, &SolverPolicy::default());

    let _ = session
        .split_focus(Axis::Y, Slot::B, 3, LeafMeta::default(), None)
        .expect("split focus");

    assert_eq!(
        session.focus_dir(Direction::Left, &snap),
        Err(NavError::StaleSnapshot)
    );
    assert_eq!(
        session.grow_focus(Direction::Right, 1, ResizeStrategy::Local, &snap),
        Err(OpError::StaleSnapshot)
    );
}

#[test]
fn resize_mutation_stales_old_snapshot() {
    let mut session = two_leaf_session();
    let root = root_rect(12, 8);
    let snap = session.solve(root, &SolverPolicy::default());

    session
        .grow_focus(Direction::Right, 1, ResizeStrategy::Local, &snap)
        .expect("resize should work with fresh snapshot");

    assert_eq!(
        session.focus_dir(Direction::Left, &snap),
        Err(NavError::StaleSnapshot)
    );
    assert_eq!(
        session.shrink_focus(Direction::Right, 1, ResizeStrategy::Local, &snap),
        Err(OpError::StaleSnapshot)
    );
}

#[test]
fn targeting_helpers_preserve_invariants_without_bumping_revision() {
    let mut session = two_leaf_session();
    let right_leaf = 2;
    session
        .set_selection(right_leaf)
        .expect("select right leaf for nested split");
    let lower_right = session
        .split_focus(Axis::Y, Slot::B, 3, LeafMeta::default(), None)
        .expect("split right leaf");
    let snap = session.solve(root_rect(12, 8), &SolverPolicy::default());
    let root = session.tree().root_id().expect("root should exist");
    let right_split = session
        .tree()
        .parent_of(right_leaf)
        .expect("nested split should exist");
    let revision = session.revision();

    session.set_selection(root).expect("select root split");
    session
        .set_focus_leaf(lower_right)
        .expect("set focus to nested leaf");
    assert_eq!(session.selection(), Some(root));
    assert_eq!(session.revision(), revision);
    session
        .focus_dir(Direction::Up, &snap)
        .expect("targeting helpers should not stale a fresh snapshot");
    session.validate().expect("state should remain valid");

    session.set_focus_leaf(1).expect("set focus to left leaf");
    assert_eq!(
        session.set_selection(right_split),
        Err(OpError::Validation(ValidationError::InvalidSelection(
            right_split,
        )))
    );
    assert_eq!(session.selection(), Some(root));
    assert_eq!(session.revision(), revision);

    session
        .set_focus_leaf(lower_right)
        .expect("restore nested focus");
    session
        .set_selection(right_split)
        .expect("select split containing focus");
    assert_eq!(session.focus(), Some(lower_right));
    assert_eq!(session.selection(), Some(right_split));
    assert_eq!(session.revision(), revision);
    session
        .validate()
        .expect("helpers should preserve invariants");
}
