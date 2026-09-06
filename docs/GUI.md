# GUI Internals — Dioxus 0.7 desktop

The GUI is a **Dioxus 0.7** desktop app (`crates/bn-app/`, crate `bn-app`): pure Rust
running in-process over the frontend-agnostic session crate
(`crates/bn-session`), rendered by the system webview (wry — the same stack
Tauri uses). There is no IPC and no serialization: components call session
ops directly through a global signal. The node canvas is a heavily-modified
fork of Dioxus/UI's workflow component (rust-ui/dioxus-ui, MIT — license in
`crates/bn-app/LICENSES/`); styling is Tailwind v4 with the shadcn theme variables.

## State model — who owns what

```
crates/bn-session (ALL document state, testable headlessly)
├── Session               doc + bridge + in-flight-job cancel slot   (session.rs)
│   ├── doc: Document     net + visual + evidence + snapshot undo    (doc.rs)
│   └── bridge: EngineBridge  compiled Engine + belief/EU caches + dirt  (engine_bridge.rs)
├── ops/*                 one pub fn per user-level action
├── views.rs              CptView + result structs, check_node, ancestor_sets
└── error.rs / jobs.rs    CmdError, job progress plumbing

crates/bn-app/src/state (global Dioxus signals — the only mutation gateway)
├── SESSION: GlobalSignal<Session>   THE application state; exec() is the
│                                    single write site (one op per gesture)
├── SELECTION                        id-keyed node/edge selection (transient)
├── DIALOG / CONTEXT_MENU            single-slot descriptors
├── MESSAGES / SHOW_MESSAGES         message log (cap 300)
├── TYPING                           not-while-typing hotkey guard
└── JOB_PROGRESS / LAST_CASE_DIR     background-job UI state

crates/bn-app/src/canvas (the node editor)
├── geometry.rs           Pt/Rect, node_size, clip_to_border, Viewport (PURE, tested)
├── scene.rs              build_scene: doc + drag overrides → world geometry (PURE, tested)
├── validation.rs         check_link (cycle/dup/utility, reconnect exemption) (PURE, tested)
├── controller.rs         VIEWPORT / GESTURE / DRAG_POS globals + reducers (fork of use_workflow)
├── canvas.rs             NetworkCanvas shell: measured viewport, world div, SVG layer, routing
├── node.rs               BnNode: belief bars, evidence clicks, connect handles
├── edge.rs               floating edges, arrowheads, hit paths, reconnect grab
├── preview.rs            connect preview line (green/red/gray)
├── context_menu.rs       node/edge/pane overlay menus
└── minimap.rs            scene-bounds minimap, click to center
```

Hard invariants (unchanged since the egui app):
- `bn_core::Network` never stores pixels; `visual` never stores probabilities.
- The UI paints ONLY from the bridge's cached beliefs/EU. A missing belief
  renders `--`. Painting never triggers inference.
- Transient gesture state (drag, rubber band, connection-in-progress) and
  selection live in UI globals, never in `Document` — undo snapshots can't
  capture mid-gesture junk, and background jobs survive canvas fiddling.
- Dialogs edit **drafts**; Apply/OK is one op = one undoable step. Cancel
  discards the draft.

## The op loop (replaces the Tauri command loop)

Every mutation follows the same shape:

```
event handler
  → copy NodeIds out of any read borrow
  → state::exec(|s| ops::xxx(s, …))         // scoped SESSION.write()
       → doc.begin_change() ONCE → mutate via Network methods
       → session.finish(dirt)               // mark + recompute if auto-update
  → write guard drops → subscribed components re-render
```

`state::exec` is the single write site: it runs ONE op, prunes stale
selection ids afterwards, and routes errors to the message log
(`exec_res` returns the error instead, for dialogs that display it inline).
**Never** write `SESSION` while holding a `.read()` guard (runtime panic),
and never hold the write guard across an `.await` (all writes live in
non-async closures; clippy `await_holding_lock` stays deny as a backstop).

`Session::finish(dirt)` marks the dirt and recomputes when
`auto_update && dirty && !net.is_empty()`. With auto-update off the status
bar shows "Stale" (`bridge.is_dirty()`) over the last cached beliefs;
Compile Now / F5 calls `ops::edit::recompute` which forces it.

## Node ids

`NodeId` is a slotmap key; keys survive `Network::clone()` (undo restore,
structure-learning swap). The UI holds real `NodeId`s — no string encoding.
Ids still go stale (selection/dialogs held across undo/delete): ops
validate with `views::check_node` (clean `BadRequest`, never a panic —
`Network::node()` panics on missing ids, so always validate first), and the
UI prunes: `state::prune_selection` runs after every op; `DialogHost` has a
close-on-vanish effect for node-bound dialogs.

## doc.rs — Document and undo

**Snapshot undo.** `begin_change()` pushes a clone of
`(net, visual, evidence)` onto the undo stack (cap 100), clears redo, sets
`modified`. One call per user-level action = one op:

- a node drag commits ONCE on release (`move_nodes`) — positions are
  optimistic in `DRAG_POS` during the drag, so the whole drag is one undo
  step;
- a dialog Apply is one op that mutates everything it needs;
- if a multi-part apply fails midway (`update_node_props`, `set_cpt`,
  `learn_cpts`), the op calls `doc.undo()` so a failed op leaves no
  half-edit.

**Generation counter.** `change_seq` bumps on `begin_change`, `undo`,
`redo`. Visual-only edits (`move_nodes`, `set_display_mode`,
`set_node_color`) use `begin_visual_change()` — undoable but no bump — so
they can't invalidate a background job. Structure learning records the seq
at prepare and the result is discarded (`Stale` error) on mismatch.

## engine_bridge.rs — the dirt system

```
enum Dirt { None < Evidence < Params < Structure }   // ordered; combine with .max()
```

| Change (op) | Dirt |
|---|---|
| toggle/set/retract finding | Evidence |
| set_cpt, parameter learning | Params |
| add/delete node/edge, move_edge, state remap, kind change, undo/redo, structure learning | Structure |
| move_nodes, set_display_mode, set_node_color, selection | None |

`recompute` recompiles the Engine when dirt ≥ Params (compile is sub-ms),
pushes the document evidence, refreshes beliefs via `all_beliefs()`, sets
`conflict` on `ConflictingEvidence` (the belief cache keeps stale values so
bars don't blank; **conflict is a normal view state, never an error**), and
— only if the net has utility nodes — fills `decision_eu` and `utility_ev`.
`last_compile_ms` feeds the status bar.

## The op surface (bn-session/src/ops/)

- **File**: `doc_new`, `doc_open(path)`, `doc_save(path?)` → `SaveResult`
  with lossy-format warnings, `refresh` (initial mount). Native file
  pickers run in the UI (`rfd`); ops take plain `PathBuf`s so they stay
  headless-testable.
- **Structure**: `add_node(kind, x, y) -> NodeId`, `add_edge`,
  `remove_edge`, `move_edge(parent, from_child, to_child)` (detach-rewire,
  one undo step), `delete_items(&[NodeId], &[(NodeId, NodeId)])`,
  `update_node_props(node, patch)` (patch carries the state list with
  `orig` remap indices), `set_network_name`.
- **Visual**: `move_nodes(&[(id, x, y)])`, `set_display_mode`, `set_node_color`.
- **CPT**: `get_cpt(node) -> CptView` (row labels precomputed via
  `row_assignment`); `set_cpt(node, data)` = begin_change + `set_table` +
  `normalize_table`.
- **Evidence**: `toggle_finding(node, state)` (click-again-retracts),
  `set_likelihood_finding(node, values)` (soft evidence),
  `retract_finding`, `retract_all_findings`.
- **Undo/compile**: `undo`, `redo`, `recompute`, `set_auto_update`.
- **Learning/tools**: `learn_cpts(path, opts)`, the structure-job trio
  (below), `simulate_cases_to_file(net, path, n, missing_pct)` (takes a
  clone so it runs off the session), `run_sensitivity(target)`,
  `solve_influence_diagram`.

Errors are `CmdError` (thiserror enum); `state::exec` logs them,
`Stale`/`Cancelled` get friendly dialog text.

## Long-running jobs (structure learning)

The UI never holds a session borrow while the job runs
(`dialogs/structure_learn.rs`):

1. `prepare_structure_job` (scoped `SESSION.write()` via `exec_res`):
   reject `Busy` if a job is running, clone the net, record `change_seq`,
   stash the cancel `AtomicBool`.
2. `run_structure_job` (`tokio::task::spawn_blocking`, no borrow): the
   worker body — read cases, learn (6 algorithms), fit parameters, format
   the report against its own clone. Progress goes through `JobCtx`
   (throttled ~20 Hz) into a tokio channel; a UI task drains the channel
   into the `JOB_PROGRESS` signal — signals are only ever written on the
   UI scheduler.
3. `apply_structure_outcome` (scoped write again): staleness check against
   the recorded seq → `Stale` error, else one `begin_change` + net
   clone-swap (NodeIds survive, so visuals/evidence stay attached) +
   `ensure_visuals` + `Dirt::Structure`.

`cancel_structure_job` flips the flag; the learners poll it via
`SearchCtrl`. `apply_structure_outcome` always clears the busy slot.

## The canvas

Three layers (mirrors the old React store separation):

1. **Pure projection** — `scene::build_scene(doc, drag_pos)` computes a
   `Scene` of world-space `NodeGeom`/`EdgeGeom` per render (via `use_memo`).
   Node sizes are **computed, never DOM-measured** (`geometry::node_size`,
   the egui `node_size` port): Utility/ExpectedValue 150×44, TitleOnly
   `max(90, chars·7.5+24)`×26, BeliefBars 180×`20+states·16+4`. Edge
   endpoints are the center→center segment clipped to each node rect
   (`clip_to_border` — floating edges).
2. **Transient controller** (`controller.rs`) — global signals: `VIEWPORT`
   (pan/zoom + MEASURED size/origin; re-measured on mount, resize, and
   fit), `GESTURE` (state machine: Idle / PendingNodeDrag / DragNodes /
   Pan / RubberBand / Connect / Reconnect / PaletteDrag), `DRAG_POS`
   (optimistic positions), `DRAG_HAPPENED` (click-vs-drag disambiguator),
   `FIT_REQUEST` (bump to fit-view). Pure reducers (`drag_update`,
   `rubber_band_world_rect`) are unit-tested.
3. **Components** — render from `SESSION` + controller, commit gestures as
   single ops.

Gesture behavior:
- Node mousedown arms `PendingNodeDrag`; >1 px of motion promotes to
  `DragNodes` and sets `DRAG_HAPPENED` (a belief-row click after a drag
  must not toggle evidence — React Flow's `nodeDragThreshold=1` parity).
  Release commits ONE `move_nodes`.
- **WebKit quirk**: spurious `mouseleave` fires on the canvas mid-drag, so
  gestures are NEVER cancelled on leave. Instead, a mousemove whose
  `held_buttons()` is empty finishes the gesture (the button was released
  outside the window). A finished/cancelled drag lands as Idle + no op,
  never a half-commit.
- Connect: 4 hover-visible handles at side midpoints (none on utility)
  start `Connect`; the whole node is the drop target (`onmouseup` on the
  wrapper). Validation is synchronous — `views::ancestor_sets` +
  `validation::check_link` (self-loop, duplicate, cycle, utility-parent,
  reconnect-original-child exemption) — and drives green/red node borders
  and the preview color. Drop on empty canvas cancels.
- Reconnect: a grab circle at the target end of a SELECTED edge starts
  `Reconnect`; drop on a valid node = `move_edge` (one undo step), drop
  back on the original child = no-op, drop on empty canvas = `remove_edge`
  (detach), invalid target = keep + log why.
- Pan: middle-drag or plain scroll wheel; zoom: ctrl/cmd+wheel (WKWebView
  delivers trackpad pinch as ctrl+wheel), clamped 0.15–4, anchored at the
  cursor. Left-drag on the pane rubber-bands (shift = additive). Fit view
  (toolbar, pane menu, doc load) fits scene bounds with 15% padding; the
  fit effect polls `get_client_rect` until the webview layout is stable.
- Palette: toolbar buttons start `PaletteDrag` (handled at the app root —
  the canvas may never see the mousedown); a ghost chip follows the
  cursor; drop inside the canvas adds the node centered at the cursor, a
  plain click adds at view center.
- Context menus: fixed panel + full-screen click-catcher (the upstream
  overlay pattern). Node: Properties…, CPT/Utility table… (hidden for
  decisions), Display-as radios, Likelihood finding… (chance only), Remove
  finding, Delete. Edge: Delete link. Pane: Add chance/decision/utility
  node here, Remove all findings, Fit view.

`BnNode` is the HTML/CSS port of `paint_node`: kind palette (chance
lemon/tan, decision blue, utility pink — exact RGB in `node.rs`), belief
bars (name · right-aligned `{:5.1}` percentage · bar; orange fill, gray on
the finding state; decision nodes show EU scaled min→max in blue), border
precedence selection > link-target green/red > finding > default, title
gets ` ⏺` with a finding. Clicking a state row → `toggle_finding`.

## Chrome, menus, shortcuts

Native menus are built with **muda** (`chrome/menu.rs`) before launch
(`Config::with_menu`); `use_muda_event_handler` routes every item by id
string through `menu::route` — one mutation path. The Auto Update check
item handle lives in a thread-local for `sync_auto_update_item`.

**Shortcut ownership rule: each shortcut has exactly one owner.**
- Native accelerators (fire even while typing — safe ops only):
  Cmd+N/O/S, Cmd+Shift+S, F5.
- Frontend hotkeys (`chrome/hotkeys.rs`, root-div keydown with the `TYPING`
  guard): Cmd+Z / Cmd+Shift+Z (check redo BEFORE undo), Cmd+A
  select-all-nodes, Delete/Backspace delete-selection, Esc (context menu →
  dialog → in-flight gesture, in that order).
- The Edit menu's Undo/Redo/Select All/Delete items deliberately have NO
  accelerators (an accelerator would fire while typing and stomp the
  webview's native text editing). The predefined Cut/Copy/Paste items are
  REQUIRED on macOS — without them Cmd+C/V/X don't work in text inputs.
- The `TYPING` flag is set by `ui::TextInput` / `ui::TextArea` focus/blur
  (and the CPT editor's cell inputs). Every free-text input MUST use those
  components — a raw `input {}` silently breaks the guard.

Chrome: `Toolbar` (palette, ⚡ Compile, auto-update, fit view, zoom %),
`StatusBar` (name + `*`, Compiled/Stale/⚠ conflict, P(findings) via
`exp(log_p_e)`, node/link counts, update ms, log toggle), `MessageLog`
(monospace, stick-to-bottom, cap 300), window title synced by a
`use_effect` on `SESSION`. File pickers are `rfd::AsyncFileDialog` in
`spawn`ed tasks; unsaved-changes confirms use `rfd::AsyncMessageDialog`.

## Dialogs

`DialogHost` renders the single `DIALOG` descriptor and closes node-bound
dialogs whose node vanished. Modal dialogs use `ui::Modal` (overlay +
centered panel). CPT editor and Sensitivity use `ui::Modeless` (no overlay,
no focus trap) so canvas evidence clicks stay live — parity with the egui
windows. Dialogs are draggable by their title bar (a pixel offset composed
onto the centering transform; while dragging, a full-screen capture layer
keeps receiving events, and a move with no held buttons ends the drag —
same trick as the canvas). The configuration dialogs (Learn CPTs, Learn
Structure, Simulate) opt out via `draggable: false`. The CPT editor keeps a draft `Vec<f64>` plus a per-focused-cell
string buffer (so "0." survives mid-typing); Normalize/Uniform are pure
functions in `logic/cpt_math.rs` (unit-tested). No virtualization — a
plain table handles editor-scale row counts; revisit past ~512 rows.

## Testing

- `crates/bn-session/src/smoke_tests.rs` — the headless suite plus
  op-layer tests (open→evidence→undo round-trip, stale-id rejection,
  `set_cpt` rollback, CPT view labels, structure-job prepare/run/apply +
  staleness, ancestor sets). No UI runtime needed.
- `bn-app` unit tests (pure logic only): `logic/format.rs`,
  `logic/cpt_math.rs`, `canvas/geometry.rs` (node sizes, clipping,
  viewport transforms, zoom clamp/anchor, fit), `canvas/scene.rs`
  (projection, drag override, bounds), `canvas/validation.rs` (the ported
  cycle/dup/utility/reconnect cases), `canvas/controller.rs` (drag/rubber
  reducers).
- Real-window e2e is deliberately absent; the logic lives in bn-session
  and the pure canvas modules where it's directly testable.
