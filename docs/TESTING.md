# Testing

Run everything: `cargo test --workspace` (all suites — engine, session,
and UI logic — must stay green).

## The testing philosophy

Two oracles anchor the whole suite:

1. **`inference/enumeration.rs`** — brute-force posterior by summing the full
   joint. Exponential, but exact by construction. Every inference feature is
   validated against it. Never delete it; route new inference code through it.
2. **Published golden values** — Asia priors from Lauritzen & Spiegelhalter,
   Sprinkler arithmetic written out by hand in test comments.

Anything stochastic uses `StdRng::seed_from_u64` — no flaky tests.

## Suite map

### bn-core unit tests (in-file `#[cfg(test)]`)

- `factor.rs`: product/marginalize vs a naive per-assignment evaluator;
  0/0 division; `from_axes` permutation; `max_out` argmax; scalar factors;
  encode/decode round-trip. **Any new factor op needs a naive-reference test
  here.**
- `model.rs`: cycle rejection, add-edge table expansion, remove-edge
  averaging, remove-node child fixing, state remap growth (+ child uniform
  fill), topo order.

### tests/common/mod.rs — shared fixtures

`sprinkler()` (hand-checkable: P(S=on)=0.30, P(R=on)=0.50),
`asia()` (8-node Chest Clinic with the standard CPTs),
`assert_close(a, b, tol, ctx)`.

### tests/inference_tests.rs

- Sprinkler priors by hand; posterior-vs-enumeration under evidence;
  explaining-away direction check.
- Asia priors vs published values (tub .0104, lung .055, bronc .45,
  either .064828, xray .11029, dysp .4360).
- Hard + likelihood evidence vs enumeration (likelihoods deliberately
  unnormalized).
- `prob_of_findings` vs an in-test hand summation.
- Conflicting evidence → typed error, and recovery after retraction.
- **The big one**: 40 seeded random nets (3–8 nodes, 2–3 states, random
  CPTs, random hard/soft evidence) — JT beliefs must equal enumeration to
  **1e-9** for every node, or both must report conflict. This catches
  essentially all junction-tree bugs. If you touch compile.rs/hugin.rs/
  factor.rs, this is the test that will tell you.
- `family_posterior` consistency with `beliefs`; `joint_posterior` on a
  non-clique pair (exercises the VE fallback) sums to 1 and marginalizes
  consistently.

### tests/feature_tests.rs

- Counting recovers Sprinkler CPTs from 10k sampled cases (< 0.02).
- EM with 30% missing recovers (< 0.08) **and log-likelihood is monotone
  every iteration** — the classic EM invariant, asserted.
- Case CSV round-trip.
- Likelihood weighting within 0.02 of exact (30k samples, seeded).
- Sensitivity on Asia: XRay must rank first for TbOrCa; results sorted;
  evidence restored.
- Umbrella influence diagram: `solve_influence_diagram` MEU and policy
  vs **brute-force enumeration of all policies** (the test enumerates all 4
  forecast→action maps itself). `expected_utilities` vs hand-computed
  conditional EU. Both check evidence save/restore.
- Format round-trips (see FILE_FORMATS.md): compare beliefs/MEU with
  tolerance 1e-12 — **never exact float equality** (reload changes topo
  order → different summation order → ~1e-16 noise; this bit us twice).

### tests/structure_tests.rs

Structure learning against forward-sampled data from known nets, compared
up to Markov equivalence via `dag_to_cpdag` (a complete graph and the truth
can have equal skeletons but different CPDAGs — always compare CPDAGs):
- Hill climbing recovers the exact Sprinkler CPDAG (20k cases); BDeu on
  Asia within skeleton Hamming distance 1 (the VisitAsia→Tuberculosis link
  is weak enough to miss at 50k).
- Determinism (same seed twice ⇒ identical edges) and cancellation (flag
  set from the progress callback ⇒ `LearnError::Cancelled`, network
  untouched).
- Learn-then-fit pipeline: HC + counting reproduce true beliefs < 0.02.
- PC-stable: exact Sprinkler skeleton, S→W←R compelled, Cloudy edges
  reversible, zero forced orientations; applied DAG is a consistent
  extension (CPDAG equals the truth's).
- Grow-Shrink: blanket contents on Asia (MB(Smoking) = {LungCancer,
  Bronchitis}); hybrid recovers every true edge, ≤ 2 extra spouse edges
  around the deterministic TbOrCa gate (faithfulness violation — this is
  expected, don't tighten the bound).
- NB star shape; TAN tree recovery from a known TAN net.
- Structural EM on 30%-missing Sprinkler data: expected-score trace
  non-decreasing (the SEM analogue of EM's monotone log-likelihood),
  skeleton within distance 1, parameters actually fitted.

Unit tests live in the `learn/structure/*` files themselves: `ln_gamma` vs
factorials, `chi2_sf` quantiles, weighted/complete-case contingency counts,
BIC by hand, **BDeu score equivalence of Markov-equivalent DAGs (1e-9)**,
G² df adjustment on degenerate tables, external-path cycle blocking,
CPDAG computation, lexicographic k-subset enumeration.

### bn-session smoke + op-layer tests (src/smoke_tests.rs, headless)

Load `examples/*.balina` (regenerate via
`cargo run -p bn-core --example make_examples` if formats change):
- open → compile → click-evidence → beliefs move the right direction →
  retract → undo cycle (both directly on Document/EngineBridge and through
  the `ops::*` layer).
- add nodes/edge → delete node → undo restores structure and table shapes.
- umbrella net populates `decision_eu` and `utility_ev`; ID solver and
  sensitivity run through their ops; `views::ancestor_sets` supports the
  canvas cycle checks.
- structure learning prepare/run/apply: busy-slot enforcement, progress
  context, staleness (`change_seq` mismatch → `Stale` error), and the
  clone-swap-undo flow (visuals survive shared SlotMap keys; one undo
  restores the original edges).
- stale node ids: retained `NodeId`s after a delete come back as
  `BadRequest` (via `views::check_node`), never a panic; undo revives them.
- `set_cpt` with a wrong-shape table rolls back (no half-edit).
- `change_seq` semantics: `begin_change`/undo/redo bump,
  `begin_visual_change` doesn't.

### UI-logic unit tests (bn-app, `#[cfg(test)]`, pure — no UI runtime)

Part of `cargo test --workspace`:
- `logic/cpt_math` — normalize/uniform/row sums/clamping/cell formatting.
- `canvas/validation` — cycle (via ancestor sets), duplicate, self-loop,
  utility-as-parent, reconnect-back-to-original, stale ids (ports of the
  old vitest cases).
- `canvas/geometry` — node sizes (the egui `node_size` port), border
  clipping, edge endpoints, viewport client↔world round-trip, zoom
  clamp + cursor-anchor invariant, fit-view centering.
- `canvas/scene` — document→geometry projection, optimistic drag override,
  scene bounds, TitleOnly sizing from title/name.
- `canvas/controller` — drag-delta zoom scaling, rubber-band rect
  normalization.
- `logic/format` — `{:5.1}` percentage alignment, `{:.4e}`, expected value.

## What is NOT covered (candidates for new tests)

- No real-window GUI interaction tests; canvas gesture wiring (drag
  promotion, connect drops, WebKit mouseleave workaround) is verified
  manually — see the gesture notes in GUI.md.
- No Alarm-scale (37-node) golden test — planned: import a published
  `alarm` XMLBIF into `tests/assets/` and cross-check vs pgmpy/bnlearn
  posteriors (tol 1e-6).
- ve_id multi-decision sequences (umbrella has one decision); add an
  oil-wildcatter fixture (2 ordered decisions, known MEU ≈ 22.5 in the
  classic parameterization) when extending the solver.
- XDSL fixtures exported by real GeNIe (we only round-trip our own output).
- Sensitivity numeric values vs an independent implementation (currently
  only ranking + sanity are asserted).
