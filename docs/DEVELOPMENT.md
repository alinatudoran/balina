# Development Guide

## Commands

```sh
cargo test --workspace                          # engine + session + UI-logic suites
cargo run -p bn-app -- examples/asia.balina     # run the editor (desktop)
cargo build --release -p bn-app                 # release build (desktop)
npm --prefix crates/bn-app run css                        # rebuild crates/bn-app/assets/main.css (Tailwind v4)
npm --prefix crates/bn-app run css:watch                  # ... in watch mode while styling
cargo run -p bn-core --example make_examples    # regenerate examples/ (repo root)
cargo clippy --workspace

# Web (wasm) build — one-time setup:
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked  # MUST match locked wasm-bindgen
# then, and again after touching learn-related bn-core/bn-session code:
sh scripts/build-worker.sh                      # builds the learning Web Worker (embedded by bn-app)
cd crates/bn-app && dx serve --web --no-default-features --features web
# type-check the wasm target without dx:
cargo check -p bn-app --no-default-features --features web --target wasm32-unknown-unknown
cargo check -p bn-worker --target wasm32-unknown-unknown
```

**Web worker gotcha**: `crates/bn-app/assets/worker/bn_worker.js` and
`bn_worker_bg.wasm` are gitignored outputs of `scripts/build-worker.sh`,
`include_bytes!`-embedded into the web app (dx only serves
manganis-referenced assets, and embedding keeps app+worker in lockstep —
cargo rebuilds the app when the artifacts change). `bn-app/build.rs` writes
empty placeholders so the crate compiles before the script has run; starting
a learning job then fails with a clear "run scripts/build-worker.sh"
message. Bump `bn_worker::proto::PROTO_VERSION` on any wire change.

First-time setup: `npm --prefix crates/bn-app install` (Tailwind CLI only — the app
itself is pure Rust; node is NOT needed to build or run, because the built
`crates/bn-app/assets/main.css` is committed and embedded via `include_str!`).
Toolchain: rustc 1.95 (edition 2024 crates). `[profile.dev]` sets
opt-level 1 (+ opt-level 2 for deps) so debug-mode inference is usable.

**Tailwind gotcha**: classes referenced in `crates/bn-app/src/**/*.rs` only exist in
the CSS after `npm --prefix crates/bn-app run css` scans the sources. A class missing
from the build fails SILENTLY (the element renders unstyled). After adding
new classes, regenerate and commit `crates/bn-app/assets/main.css`.

## Dependency decisions & pins (important)

| Crate / package | Note |
|---|---|
| `dioxus` 0.7 (`desktop`) | The whole UI. Desktop renders in the system webview (wry); Rust runs in-process — no IPC. Signals: never write one while holding its read guard; never hold `SESSION.write()` across `.await`. |
| `rfd` | Native file/message dialogs, called directly from `spawn`ed tasks (no plugin layer). |
| muda (via `dioxus::desktop::muda`) | Native menu bar — same crate Tauri used, so the macOS menu gotchas carry over verbatim. Use the re-export, don't add a separate `muda` dep (version skew breaks the types). |
| `tokio` | `spawn_blocking` for structure learning / simulation; the runtime itself comes with dioxus-desktop. |
| Tailwind 4 (dev-only, via npm) | `crates/bn-app/tailwind.css` input with the shadcn theme vars; output committed at `crates/bn-app/assets/main.css`. |
| `rand` **0.10** | `random()`/`random_range()` moved to the **`RngExt`** trait — `use rand::{Rng, RngExt};` where you call them. Seeded tests: `StdRng::seed_from_u64`. |
| `slotmap` | node identity; `SecondaryMap` for everything keyed by NodeId. The UI holds real `NodeId`s (validate with `views::check_node`). |
| `quick-xml` 0.41 | manual events only (no serde feature); `reader.config_mut().trim_text(true)`; text via `t.decode()`. |
| NOT used, on purpose | `petgraph`, `ndarray` (rationale in ARCHITECTURE.md); serde in bn-session (no wire boundary anymore); Dioxus/UI as a dependency — its workflow component is FORKED into `crates/bn-app/src/canvas/` (MIT, license in `crates/bn-app/LICENSES/`), not consumed as a crate. |

Platform note: building `bn-app` on Linux needs the webkit2gtk stack
(`libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev librsvg2-dev` on
Debian/Ubuntu). macOS and Windows need nothing extra.

## Rust/Dioxus gotchas hit in this codebase

- rustc 1.95 rejects `|(_, &d)|`-style patterns on `&(_, _)` tuples
  ("cannot explicitly dereference within an implicitly-borrowing pattern") —
  write `|&(_, &d)|`.
- `GlobalSignal` writes go through `*SIG.write() = v` (a `static` can't be
  borrowed mutably for `.set()`); reads need `dioxus::prelude::ReadableExt`
  in scope when the prelude isn't glob-imported.
- `if let … = SIG.read().clone() { body }` keeps the read guard alive
  through the body (Rust 2024 if-let scrutinee scoping), so a body that
  writes the same signal panics with `AlreadyBorrowed` at runtime. Bind
  first: `let v = SIG.read().clone();` then `if let`/`match` on `v`.
  (Plain `if COND {}` conditions are safe — their temporaries drop before
  the block.)
- rsx format segments don't take arbitrary expressions — no inline
  `if`/method chains inside `"{…}"`; precompute into a local first.
- `document::Style` must live in a component that never re-renders
  (diffing its props is unsupported and warns) — see `app.rs::Stylesheet`.
- The webview reports a collapsed rect right after mount — anything that
  needs measured geometry (fit-view) polls `get_client_rect()` until two
  consecutive reads agree (see the fit effect in `canvas.rs`).
- WebKit fires spurious `mouseleave` on the canvas during node drags; the
  gesture system ignores leave entirely and instead treats a mousemove
  with empty `held_buttons()` as the release (see GUI.md).
- macOS delivers trackpad pinch as ctrl+wheel; plain wheel pans, ctrl/cmd+
  wheel zooms.

## Conventions

- **Layout convention** (parents outer, self fastest; sorted Factor vars) is
  defined once in [ENGINE.md](ENGINE.md). Everything — CPT editor rows, file
  formats, EM count folding — assumes it. Do not introduce a second layout.
- Every structural mutation goes through `Network` methods (they keep table
  shapes consistent). Never mutate `Node.parents`/`Node.table` fields
  directly.
- One user gesture = one op = one `doc.begin_change()` before mutating,
  finished with `Session::finish(dirt)` (Dirt table in GUI.md). Forgetting
  begin_change = broken undo; forgetting Dirt = stale bars.
- UI code only mutates the session through `state::exec` / `exec_res` —
  never `SESSION.write().doc` directly.
- All free-text inputs are `ui::TextInput`/`ui::TextArea` (they maintain
  the `TYPING` hotkey guard).
- Errors: engine errors are typed (`thiserror`) and surface as `CmdError`;
  the UI logs them, never crashes. `ConflictingEvidence` (P(e)=0) is a
  normal user-visible state, not a bug and not an error.
- Tests comparing floats after any save/load or re-compile use tolerances
  (1e-12), never exact equality.

## Debugging tips

- Inference wrong? Run
  `cargo test -p bn-core --test inference_tests junction_tree_equals` —
  the 40-seed differential test localizes JT bugs immediately. Shrink by
  hardcoding the failing seed in `random_net`.
- Factor op wrong? The naive evaluators in `factor.rs` tests are the ground
  truth; add a case there.
- Op behaving oddly? Drive it headlessly: build a `bn_session::Session`
  in a test and call `ops::*` directly (see
  `crates/bn-session/src/smoke_tests.rs`) — no UI runtime involved.
- Canvas geometry weird? The projection is pure — add a case to
  `canvas/scene.rs` or `canvas/geometry.rs` tests.
- Element renders unstyled? You forgot `npm --prefix crates/bn-app run css` after
  adding a Tailwind class.
- Webview devtools: right-click → Inspect Element in a debug build shows
  the DOM, computed styles, and console.
