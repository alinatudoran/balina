# Balina — project context for coding sessions

Bayesian network / influence diagram editor. Three parts:
`crates/bn-core` (engine library, zero GUI deps), `crates/bn-session`
(application session: document + undo + engine bridge + op bodies, no GUI
deps), and `crates/bn-app/` (Dioxus 0.7 desktop app, crate `bn-app`: pure-Rust UI over
the system webview; the canvas is a fork of Dioxus/UI's workflow component).
Full developer docs live in `docs/` — **start with `docs/ARCHITECTURE.md`**;
`docs/ROADMAP.md` lists what to build next.

## Commands

- Test: `cargo test --workspace` (must stay green)
- Run: `cargo run -p bn-app -- examples/asia.balina`
- Rebuild CSS after adding/changing Tailwind classes in `crates/bn-app/src/**/*.rs`:
  `npm --prefix crates/bn-app run css` (output `crates/bn-app/assets/main.css` is committed;
  first time: `npm --prefix crates/bn-app install`)
- Build release: `cargo build --release -p bn-app` (bare binary, no app icon);
  desktop bundle with icon: `dx bundle --release` from `crates/bn-app/`
  (config in `crates/bn-app/Dioxus.toml`; output under `target/dx/balina/bundle/`)
- Regenerate examples: `cargo run -p bn-core --example make_examples`

## Critical conventions (breaking these breaks everything)

- **Table/factor layout**: row-major, LAST axis fastest; CPT axes are
  `[parent_0..parent_k, self]` in `Node.parents` order, so each contiguous
  run of `out_card` values is one conditional distribution. All file formats
  share this ordering. `Factor.vars` is always sorted ascending
  (`Factor::from_axes` permutes). Details: `docs/ENGINE.md`.
- All structural edits go through `Network` methods (they auto-reshape
  tables); never mutate `Node` fields directly.
- **One user gesture = exactly one `ops::` call** through `state::exec` /
  `exec_res`; UI code never mutates `SESSION.write().doc` directly. Each op
  calls `doc.begin_change()` exactly once BEFORE mutating (snapshot undo)
  and ends with `Session::finish(dirt)` with the correct `Dirt` level
  (Evidence < Params < Structure) — table in `docs/GUI.md`. Visual-only
  edits use `begin_visual_change()` (undoable, no `change_seq` bump).
- **Signal borrow discipline**: never write a global signal while holding a
  `.read()` guard on it — copy `NodeId`s out first, then call `exec`. Never
  hold `SESSION.write()` across an `.await` (every write lives inside a
  non-async closure). Trap: `if let … = SIG.read().clone() { body }` keeps
  the read guard alive through the body (Rust 2024 scrutinee scoping) —
  always `let v = SIG.read().clone();` FIRST, then match on `v`, if the
  body writes the same signal (runtime AlreadyBorrowed panic otherwise).
- Node ids are real `NodeId`s in-process. They can still go stale (undo,
  delete, learning): ops validate with `views::check_node` (clean
  BadRequest); `Network::node()` panics on stale ids. Transient state
  (selection, dialogs) is pruned via `state::prune_selection` after every op
  and the DialogHost close-on-vanish effect.
- The UI is a pure view of `SESSION`: components read `doc`/`bridge` state
  in render; rendering never runs inference. The canvas projects geometry
  through `canvas::scene::build_scene` (pure, unit-tested); gesture/viewport
  transients live in `canvas::controller` globals and never enter the
  document.
- `inference/enumeration.rs` is the correctness oracle — never delete it;
  validate new inference features against it (see the 40-seed differential
  test in `tests/inference_tests.rs`).
- P(evidence)=0 is a typed `ConflictingEvidence` error inside the engine and
  a normal UI state (`EngineBridge::conflict`), never a panic, NaN, or
  `CmdError`.

## Environment gotchas

- Long jobs (structure learning): prepare (scoped `SESSION.write()`) →
  `tokio::task::spawn_blocking` (no borrow held) → apply (scoped write)
  with a `change_seq` staleness check; progress flows through a channel
  into `JOB_PROGRESS` (signals are only written on the UI scheduler).
- Native menu accelerators fire even while typing — only Cmd+N/O/S,
  Cmd+Shift+S and F5 are menu accelerators; Cmd+Z/Cmd+A/Delete are frontend
  hotkeys guarded by the `TYPING` flag. ALL free-text inputs must be
  `ui::TextInput`/`ui::TextArea` (they set `TYPING`); a raw `input {}`
  silently breaks the guard. The predefined Cut/Copy/Paste menu items are
  required on macOS or clipboard shortcuts break in text inputs.
- WebKit fires spurious `mouseleave` on the canvas mid-drag — gestures are
  never cancelled on leave; instead a mousemove with `held_buttons()` empty
  finishes the gesture (release happened outside the window).
- Tailwind classes used in Rust only take effect after
  `npm --prefix crates/bn-app run css` — a missing class fails silently (element
  renders unstyled). Regenerate + commit `crates/bn-app/assets/main.css`.
- rand 0.10: `random`/`random_range` need `use rand::{Rng, RngExt};`.
- Float comparisons after save/load or recompile use tolerances (1e-12),
  never exact equality (summation-order noise).
- Linux builds of `bn-app` need webkit2gtk system packages (macOS/Windows
  need nothing extra).
