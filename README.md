# libtiler

`libtiler` is a deterministic binary split tiling library for editor and TUI layouts.

It implements the design in [`docs/001.design_spec_v1.md`](docs/001.design_spec_v1.md):

- single-root binary split trees
- exact half-open integer rectangles
- pure best-effort solver with strict feasibility certification
- hard leaf min/max constraints and shrink priorities
- snapshot-gated geometry operations
- structural edits, presets, navigation, and edge-eligible resize

## Crate model

The library is intentionally split into two layers.

- Core: topology, metadata, validation, summaries, and solving
- Session: focus, selection, geometry-driven commands, and revision tracking

Public modules are re-exported from `libtiler` for ergonomic use.

## Example

```rust
use libtiler::{
    Axis, Direction, LeafMeta, Rect, ResizeStrategy, Session, Slot, SolverPolicy,
};

let mut session = Session::new();
let _a = session.insert_root("main", LeafMeta::default())?;
let _b = session.split_focus(Axis::X, Slot::B, "side", LeafMeta::default(), None)?;
let _c = session.wrap_selection(Axis::Y, Slot::B, "log", LeafMeta::default(), None)?;

let root = Rect { x: 0, y: 0, w: 120, h: 40 };
let snap = session.solve(root, &SolverPolicy::default());
session.focus_dir(Direction::Right, &snap)?;
session.grow_focus(Direction::Down, 4, ResizeStrategy::Local, &snap)?;

let solved = session.solve(root, &SolverPolicy::default());
assert!(solved.strict_feasible);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Presets and rebalancing

Included subtree rebuild presets:

- `Balanced`
- `Dwindle`
- `Tall`
- `Wide`

Included rebalance modes:

- `BinaryEqual`
- `LeafCount`

## Validation and testing

The crate ships with:

- exact allocator oracle checks
- reference-solver comparisons
- raster partition proofs
- brute-force summary envelope checks
- symmetry and roundtrip regression tests
- end-to-end session mutation, navigation, preset, and resize coverage

Run the full suite with:

```bash
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
