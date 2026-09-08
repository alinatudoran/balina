# Balina — project context for coding sessions

Bayesian network / influence diagram editor. Four parts:
`crates/bn-core` (engine library, zero GUI deps), `crates/bn-session`
(application session: document + undo + engine bridge + op bodies, no GUI
deps), `crates/bn-app/` (Dioxus 0.7 app, crate `bn-app`: pure-Rust UI that
compiles BOTH as a desktop app over the system webview — default — and as a
browser wasm app via `--features web`; the canvas is a fork of Dioxus/UI's
workflow component), and `crates/bn-worker` (structure learning compiled to
a standalone wasm module that the web build runs in a Web Worker).
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
- Web build: `sh scripts/build-worker.sh` FIRST (worker artifacts are
  gitignored; needs `rustup target add wasm32-unknown-unknown` and
  `cargo install wasm-bindgen-cli --version 0.2.128 --locked` — the CLI
  version must match the locked `wasm-bindgen` or the worker fails at
  runtime), then from `crates/bn-app/`:
  `dx serve --web --no-default-features --features web`
- Wasm type-check (part of keeping the build green):
  `cargo check -p bn-app --no-default-features --features web --target wasm32-unknown-unknown`
  (and `-p bn-worker --target wasm32-unknown-unknown`)
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

## Dual-target rules (desktop + web from one `bn-app` crate)

- Code gates on `#[cfg(target_arch = "wasm32")]`; the cargo features
  `desktop`/`web` ONLY pick the Dioxus renderer. Platform-specific deps live
  in `[target.'cfg(...)'.dependencies]` tables.
- Anything that touches paths, pickers, fs, window, timers, or the blocking
  pool goes through `crate::platform::` (`platform/desktop.rs` vs
  `platform/web.rs`, same surface: `CaseFile`, `pick_case_file`, `sleep_ms`,
  `set_window_title`, `confirm`, `download`, `run_structure_job`,
  `apply_structure_outcome`). Don't call `tokio`/`rfd`/`std::fs` from shared
  UI code directly.
- NodeIds are slotmap keys and DO NOT survive serialization. The web worker
  therefore returns a by-name `bn_session::patch::StructurePatch`, applied
  via `apply_structure_patch` (equivalence with the desktop clone-swap is
  covered by a smoke test). Never ship a `Network` across the worker
  boundary and assign it into the document.
- The worker protocol lives in `bn-worker/src/proto.rs`; bump
  `PROTO_VERSION` on any wire change. The worker artifacts are gitignored,
  built by `scripts/build-worker.sh`, and `include_bytes!`-EMBEDDED into the
  web app (dx only serves manganis-referenced assets; embedding also keeps
  app+worker in lockstep — cargo rebuilds the app when the artifacts
  change). Re-run the script after touching learn-related
  bn-core/bn-session code; before it has ever run, `bn-app/build.rs` writes
  empty placeholders and starting a job fails with a clear message.
- `std::time::Instant` panics on wasm — use `web_time::Instant` in
  bn-session (re-exports std on native). rand needs `getrandom/wasm_js` on
  wasm (already wired in bn-session's target table).
- Web menu = `chrome::menu_bar` (in-app), routing through the SAME
  `chrome::menu::route(id)` ids as the native muda menu — add new menu
  items in both places. Recent files are desktop-only (paths are
  meaningless in a browser); saves/exports on web are blob downloads.

## Environment gotchas

- Long jobs (structure learning): prepare (scoped `SESSION.write()`) →
  `platform::run_structure_job` (desktop: `spawn_blocking`; web: Web Worker,
  cancel = `terminate()`) → apply (scoped write) with a `change_seq`
  staleness check; progress flows into `JOB_PROGRESS` (signals are only
  written on the UI scheduler).
- Native menu accelerators fire even while typing — only Cmd+N/O/S,
  Cmd+Shift+S and F5 are menu accelerators; Cmd+Z/Cmd+A/Delete are frontend
  hotkeys guarded by the `TYPING` flag (on web, Cmd+N/O/S/F5 are frontend
  too, wired before the guard in `chrome::hotkeys`). ALL free-text inputs
  must be `ui::TextInput`/`ui::TextArea` (they set `TYPING`); a raw
  `input {}` silently breaks the guard. The predefined Cut/Copy/Paste menu
  items are required on macOS or clipboard shortcuts break in text inputs.
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
