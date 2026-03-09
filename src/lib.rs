#![forbid(unsafe_code)]

pub mod error;
pub mod geom;
pub mod ids;
pub mod limits;
pub mod nav;
pub mod preset;
pub mod resize;
pub mod session;
pub mod snapshot;
pub mod solver;
pub mod tree;

pub use error::{NavError, OpError, SolveError, ValidationError};
pub use geom::{Axis, Direction, Rect, Slot};
pub use ids::{NodeId, Revision};
pub use limits::{LeafMeta, Priority, SizeLimits, Summary, WeightPair, canonicalize_weights};
pub use preset::{BalancedPreset, DwindlePreset, PresetKind, TallPreset, WidePreset};
pub use resize::ResizeStrategy;
pub use session::{RebalanceMode, Session};
pub use snapshot::{ScoreTuple, Snapshot, SplitTrace, Violation, ViolationKind};
pub use solver::{
    OverflowMode, PairSpec, ShortageMode, SolverPolicy, TieBreakMode, choose_extent,
    choose_extent_with_score, score, solve, solve_strict, solve_strict_with_revision,
    solve_with_revision, summarize,
};
pub use tree::{LeafNode, Node, SplitNode, Tree};
