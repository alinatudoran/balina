# Balina — Architecture Overview

Balina is a Bayesian network / influence diagram editor:
a pure-Rust inference engine (`bn-core`), a frontend-agnostic application
session (`bn-session`), and a **Dioxus 0.7 desktop app** (`crates/bn-app/`, crate
`bn-app`) — the whole application is Rust in a single process; the UI
renders in the system webview. This document is the map; the other docs go
deep:

| Doc | Contents |
|---|---|
| [ENGINE.md](ENGINE.md) | bn-core internals: model, factor algebra, junction tree, learning, decisions |
| [GUI.md](GUI.md) | GUI internals: session signal, ops, dirt system, canvas, dialogs |
| [FILE_FORMATS.md](FILE_FORMATS.md) | `.balina` JSON, XMLBIF, XDSL |
| [TESTING.md](TESTING.md) | test suites, oracles, how to add tests |
| [DEVELOPMENT.md](DEVELOPMENT.md) | build commands, dependency pins, API gotchas |
| [ROADMAP.md](ROADMAP.md) | known limitations and planned work, with implementation hints |

## Workspace layout

```
balina/
├── Cargo.toml                  # [workspace] members; dev profile opt-level tuning
├── README.md                   # user-facing overview & quick tour
├── docs/                       # ← you are here
├── examples/                   # asia.balina, umbrella.balina, asia.xmlbif, umbrella.xdsl
│                               #   (regenerate: cargo run -p bn-core --example make_examples)
├── crates/
│   ├── bn-core/                # engine LIBRARY — zero GUI deps, compiles in seconds
│   │   ├── src/
│   │   │   ├── lib.rs          # module tree + re-exports
│   │   │   ├── error.rs        # thiserror enums: ModelError, InferenceError, IoError, ...
│   │   │   ├── model.rs        # Network / Node / State / Table + structural invariants
│   │   │   ├── factor.rs       # Factor algebra over dense VarIds (THE hot path)
│   │   │   ├── inference/
│   │   │   │   ├── mod.rs      # Engine facade, Evidence, Finding
│   │   │   │   ├── compile.rs  # CompiledNet (NodeId→VarId), moralize, min-fill, JunctionTree
│   │   │   │   ├── hugin.rs    # collect/distribute message passing, P(e), conflict
│   │   │   │   └── enumeration.rs  # brute-force oracle (KEEP FOREVER — differential tests)
│   │   │   ├── decision/       # single.rs: EU per choice; ve_id.rs: exact MEU + policies
│   │   │   ├── learn/          # cases, counting, EM, structure/ (6 algorithms, SearchCtrl)
│   │   │   ├── sample/         # forward sampling, generate_cases, likelihood weighting
│   │   │   ├── sensitivity.rs  # mutual info / entropy reduction / variance reduction
│   │   │   └── io/             # Document { network, visual }; native/xmlbif/xdsl
│   │   ├── examples/make_examples.rs   # writes examples/ at repo root
│   │   └── tests/              # golden + differential + feature + structure suites
│   ├── bn-session/             # application session LIBRARY — no GUI deps
│   │   └── src/
│   │       ├── doc.rs          # Document: net+visual+evidence, snapshot undo, Dirt, change_seq
│   │       ├── engine_bridge.rs# EngineBridge: dirty flags, belief cache, EU computation
│   │       ├── session.rs      # Session { doc, bridge, job slot }; finish(dirt)
│   │       ├── ops/            # one pub fn per user-level action (file/edit/evidence/cpt/learn/tools)
│   │       ├── views.rs        # precomputed views (CptView, results), check_node, ancestor_sets
│   │       ├── error.rs        # CmdError
│   │       ├── jobs.rs         # JobCtx: cancel flag + throttled progress sink
│   │       └── smoke_tests.rs  # headless op-layer tests (#[cfg(test)])
│   └── bn-app/                 # Dioxus 0.7 desktop app
│       ├── Cargo.toml          # dioxus (desktop), rfd, tokio; clippy await_holding_lock = deny
│       ├── package.json        # Tailwind v4 CSS build ONLY (npm run css); the app is pure Rust
│       ├── tailwind.css        # Tailwind input (+ shadcn theme vars)
│       ├── assets/main.css     # COMMITTED build output (embedded via include_str!)
│       ├── LICENSES/           # MIT license of the forked Dioxus/UI workflow component
│       └── src/
│           ├── main.rs         # window + muda menu config, CLI file arg, launch
│           ├── app.rs          # root layout, menu routing, title sync, palette drag, hotkey host
│           ├── state/          # SESSION signal + exec(), SELECTION, DIALOG, MESSAGES, TYPING
│           ├── canvas/         # the node editor (fork of Dioxus/UI's workflow component):
│           │                   #   geometry/scene/validation (pure) · controller (gestures)
│           │                   #   canvas/node/edge/preview/context_menu/minimap (components)
│           ├── chrome/         # menu (muda), toolbar, status bar, message log, hotkeys, file ops (rfd)
│           ├── dialogs/        # host + node props, CPT editor, learning, simulate, sensitivity, misc
│           ├── ui/             # minimal UI kit (Modal, Modeless, TextInput with TYPING guard)
│           └── logic/          # pure helpers: format.rs, cpt_math.rs (unit-tested)
```

## Data-flow in one paragraph

`bn_core::Network` is the single source of truth for structure and CPTs;
`bn-session`'s `Document` wraps it with visual layout, evidence, and
snapshot undo, and `Session` pairs that with the `EngineBridge` caches. The
UI holds the `Session` in one global Dioxus signal (`state::SESSION`).
Every user gesture is exactly one `ops::` call through `state::exec`: the
op calls `doc.begin_change()` (pushes an undo snapshot), mutates through
`Network`'s invariant-keeping methods, and ends with
`Session::finish(dirt)`, which recompiles/repropagates the junction-tree
`Engine` if dirty (and auto-update is on) and refreshes the belief/EU
caches. Dropping the write guard re-renders every subscribed component —
the in-process equivalent of the old full-snapshot swap. The canvas renders
exclusively from session state projected through the pure
`canvas::scene::build_scene`; rendering never computes probabilities.

## Design decisions worth remembering

- **No petgraph**: the DAG is `parents: Vec<NodeId>` per node + a derived
  children map. BN semantics need an *ordered* parent list tied to CPT axes;
  petgraph doesn't model that. Cycle check / topo sort are ~50 lines each.
- **No ndarray**: factors are flat `Vec<f64>` with hand-computed strides and
  an odometer loop. Dynamic-rank ops through `ArrayD` are slower and clumsier.
- **bn-session is UI-free on purpose**: every op body is a plain function on
  `&mut Session`, so the whole application layer tests headlessly
  (`cargo test -p bn-session`) and the UI calls the same functions the tests
  do. There is **no IPC/serialization layer** — the Dioxus UI runs in-process
  and holds real `NodeId`s (the old Tauri DocView/string-id DTO layer was
  deleted in the Dioxus migration; see git tag `pre-dioxus`).
- **One coarse session signal, not fine-grained state**: a full subscriber
  re-run per action is cheap at editor scale (propagation is sub-ms, nets
  are tens of nodes), and it keeps the "UI is a pure view" invariant simple.
  `use_memo` (e.g. the scene projection) gates propagation where output is
  unchanged.
- **The canvas is a gutted fork, not a library**: Dioxus/UI's workflow
  component (rust-ui/dioxus-ui, MIT) provided the event-wiring skeleton and
  pan/zoom math; its index-keyed node state, internal undo and clipboard
  were removed — the document owns structure/positions, the controller owns
  only transients (gesture state machine, viewport, optimistic drag
  positions), keyed by `NodeId`.
- **Snapshot undo, not command pattern**: editor-scale networks are KBs;
  snapshots make every edit — including state remaps that reshape child
  CPTs — trivially reversible. Revisit only if someone loads megabyte-scale
  learned nets (see ROADMAP.md).
- **Synchronous inference inside the op**: propagation is sub-millisecond at
  editor scale (the status bar shows the measured time). Long-running work
  (structure learning) runs prepare → `spawn_blocking` → apply, never
  holding a session borrow while working; a `Document::change_seq`
  generation counter discards results that raced a concurrent edit. CPT
  learning and sensitivity are still synchronous (ROADMAP item 5).
- **Utility nodes carry no variable** in the compiled net; their parents are
  married during moralization so every utility family lands inside one clique
  (that's what makes `Engine::joint_posterior(parents)` a single clique read).
- **`enumeration.rs` is load-bearing**: it is the correctness oracle for the
  junction tree. Never delete it, and route new inference features through
  differential tests against it.
