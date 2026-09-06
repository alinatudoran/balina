# Roadmap & Known Limitations

Status: v0.2 — full engine (inference, parameter + structure learning,
sampling, sensitivity, decisions, 3 file formats) + Dioxus 0.7 desktop
editor (pure Rust, in-process — migrated from Tauri 2 + React; see git tag
`pre-dioxus`) with belief bars, CPT/properties dialogs, undo, soft
evidence, learning/sensitivity/ID tooling. Structure learning (hill
climbing BIC/BDeu, Grow-Shrink hybrid, PC-stable, Naive Bayes/TAN,
Structural EM) runs on a blocking thread with channel progress + cancel.
`cargo test --workspace` green.

This file lists what was consciously deferred, with enough implementation
hints to pick each item up cold. Ordered roughly by value/effort.

## High value, moderate effort

### 1. Copy / paste / duplicate of subgraphs
Missing entirely (Edit menu has no clipboard). Plan: new bn-session ops
`copy_nodes(ids) -> Clipboard` (clone selected nodes + internal edges +
CPTs) and `paste(clip)` (insert with `fresh_name`-style dedup — suffix
`_1`, `_2`…, offset positions by +(20,20), one `begin_change`). External
parents are dropped (paste contracts tables automatically since edges to
outside aren't recreated). The clipboard can be a plain Rust struct held in
a UI global; serialize it (native-JSON node shapes) when cross-document
paste lands with multi-doc tabs.

### 2. Persist evidence + camera in `.balina`
The native format gains `#[serde(default)] evidence` and
`camera: Option<(f32, f32, f32)>` (x, y, zoom). Load path:
`Document::from_io_document` applies them; add a `set_camera` visual-no-undo
op that stores the viewport on `Document` after pan/zoom settles. Backward
compatible via defaults. Users will expect findings to persist with the file.

### 3. Alarm-scale golden test + LOD polish
Download the standard 37-node `alarm` XMLBIF into `crates/bn-core/tests/assets/`,
cross-check ~5 posteriors against pgmpy/bnlearn (tol 1e-6). Then profile the
canvas at 40+ nodes; if belief-bar DOM cost shows up, add level-of-detail in
`BnNode` (skip bar text below ~50% zoom via `VIEWPORT`, render TitleOnly
below ~30%), and cull nodes whose rect misses the viewport in the render
loop (the scene already has every rect).

### 4. Likelihood-finding polish
The soft-evidence dialog exists (node context menu → Likelihood finding…).
Missing the distinct rendering: dashed border + partial gray bars for nodes
with a likelihood finding (`doc.evidence.get(id)` distinguishes
`Finding::Likelihood`; it's a `canvas/node.rs` styling change).

## Medium value

### 5. Progress/cancel for CPT learning and sensitivity
`learn_cpts` and `run_sensitivity` run synchronously on the session (fine
at editor scale). For long EM runs: give `learn_em` a per-iteration
progress callback (like the structure learners' `SearchCtrl`), then move
the op to the prepare/run/apply pattern in `ops/learn.rs` with a progress
channel into `JOB_PROGRESS` — the plumbing (jobs.rs, busy slot, staleness
check) already exists.

### 5b. Structure learning polish
Deferred from the initial implementation: tabu list for hill climbing
(restarts cover plateaus for now), exposing `grow_shrink_blankets` as an
analysis view, `TanOptions::root` selection, rendering PC-stable's
reversible edges with a distinct stroke on the canvas (the report/warnings
carry them already), and a `min_obs_per_cell` control in the dialog.

### 6. Incremental engine updates
`Engine::refresh_potentials` exists (same-structure CPT refresh) but
`EngineBridge::recompute` always recompiles on `Dirt::Params`. Wire it:
Params → refresh_potentials + propagate; Structure → recompile. Only worth
it when compile time shows up in `last_compile_ms` on big nets. Further:
Hugin fast retraction (divide evidence out instead of re-initializing) —
only if profiling demands.

### 7. Deterministic (function) nodes
Deterministic nodes = CPT rows that are indicator vectors. Minimal
version: CPT-editor toggle that switches each row to a state-selector combo
writing 0/1 rows (the 0/0 division convention already handles zeros in the
JT). No engine change needed.

### 8. Sensitivity & EU fast paths / parallelism
`sensitivity_to_findings` propagates per candidate-state. Fast path when
target and candidate share a clique: read `joint_posterior` directly (the
method exists — compare clique membership via `JunctionTree::clique_containing`).
Then a `parallel` feature with rayon: per-candidate loop with cloned engines;
same for EM's E-step (dedup already done; cases are independent).

## Nice to have

### 9. Multi-document tabs
`Session` is the natural seam: make it `Vec<DocState>` + active index (or
`HashMap<DocId, DocState>`), add an optional `doc_id` param to ops, and a
tab strip component above the canvas. `Document` + `EngineBridge` are
already self-contained per document.

### 10. Unsaved-changes confirmation on quit
New/Open prompt via `rfd::AsyncMessageDialog` (`chrome/file_ops.rs`), but
closing the window doesn't. Hook the window close-requested event
(`dioxus::desktop` window event handler) and show the same confirm when
`doc.modified`.

### 11. DNE format
Text `.dne` reader/writer would unlock Norsys's library of published
networks. Grammar is a nested `key = value / block` structure; a hand-rolled
recursive-descent parser in `io/dne.rs` following the xmlbif.rs DTO pattern.

### 12. Command-pattern undo (only if needed)
Snapshot undo clones the net per edit. If someone loads megabyte CPTs
(learned nets with many parents), memory will hurt. The original design doc
(git history / plan file) has the full Command enum sketch with dirt
classification. Don't do this preemptively.

### 13. ve_id refinements
- Policy-domain pruning: drop domain variables the argmax doesn't depend on
  (compare argmax slices across each domain axis; if constant, marginalize
  the axis out of `Policy::best`). Keeps big policy tables readable.
- Multi-decision fixtures (oil wildcatter) in tests.
- Evidence-downstream-of-decision correctness (see caveat in ENGINE.md):
  switch the max criterion to (Σ p·u)/(Σ p) per decision configuration.

### 14. Misc UI polish
- Belief-bar animation (CSS transition on the fill width, ~120 ms).
- Node color picker (`set_node_color` op + persistence exist; no UI sets
  it — add a color swatch row to the node context menu).
- Export canvas as PNG (render the scene to an offscreen canvas via
  `document::eval`, or rasterize the SVG layer).
- Keyboard nudge of selected nodes (arrow keys → `move_nodes` — the
  upstream workflow component had this; re-add in `chrome/hotkeys.rs`).
- P(e) per-finding breakdown tooltip.
- Menu enabled-state sync (grey out Undo/Redo via `MenuItem::set_enabled`
  from a `use_effect` — they are currently always enabled and no-op
  safely).

## Explicitly rejected (don't re-litigate without new evidence)

- petgraph / ndarray — see ARCHITECTURE.md "Design decisions".
- Third-party BN crates (`loopybayesnet`, `reCTBN`, `rsbn`) — evaluated
  before building; none fit (approximate-only / continuous-time / unpublished
  research code). `rsbn` (github.com/neuppl/rsbn, BSD-3) may be consulted as
  a *reading reference* for exact inference, per user request.
- Dioxus/UI's workflow component as a dependency — its state model is
  index-keyed and owns structure/undo, which conflicts with the document;
  it is forked (gutted to a gesture/viewport controller) instead. Same for
  the Blitz/native renderer: not production-ready; the app targets the
  system webview.
- Real-window e2e — logic is tested headlessly in bn-session and the pure
  canvas modules instead.
