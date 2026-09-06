# bn-core — Engine Internals

Everything in the engine hangs off two conventions. Read this section before
touching any file in `bn-core`.

## Convention 1: table/factor memory layout

**Row-major, LAST axis varies fastest.** A CPT's axes are
`[parent_0, ..., parent_k, self]` in the node's `parents` order. Therefore
each contiguous run of `out_card` entries is one conditional distribution
P(self | one parent configuration) — which is exactly one row in the CPT
editor and trivial to normalize.

- `Node::out_card()` = 1 for utility nodes, `states.len()` otherwise.
- `Network::table_len(id)` = `row_count(id) * out_card` where
  `row_count = Π parent cards`.
- `Network::row_assignment(id, row)` decodes a row index into parent state
  indices (last parent fastest) — the CPT editor uses this for row labels.
- `factor::encode_index / decode_index` are the mixed-radix helpers.
- XMLBIF and XDSL both list probabilities in this same order (parents outer,
  child fastest), so file I/O copies table data verbatim.

## Convention 2: sorted factor variables

`Factor { vars: Vec<VarId>, cards, data }` — `vars` is **always sorted
ascending**. `Factor::from_axes(axes, cards, data)` takes data in arbitrary
axis order (e.g. a CPT's `[parents..., self]`) and permutes it into canonical
sorted order. `VarId`s are dense indices assigned in topological order by
`CompiledNet::compile`, so a node's VarId is always greater than its parents'.

All factor ops (product, marginalize, divide, max_out) use the same pattern:
precompute per-axis strides of each operand *within the result's axis list*
(stride 0 for absent variables), then walk the result linearly while an
odometer counter updates operand offsets incrementally. If you add a new
factor op, copy that pattern and add a test against the naive tuple-
enumeration reference in `factor.rs`'s test module.

Special semantics: `divide_assign` uses the Hugin convention **0/0 = 0**
(required for deterministic CPTs). `normalize()` returns the pre-normalization
sum — that's how P(e) is read.

## model.rs — Network invariants

All structural edits go through `Network` methods, which keep every table
shaped consistently:

| Edit | Table effect |
|---|---|
| `add_edge(p, c)` | new parent appended **last**; each old row of c's table repeated k times |
| `remove_edge(p, c)` | c's table **averaged** over p's axis |
| `remove_node(id)` | removes edges first (children's tables contract), then the node |
| `remap_states(id, new_states, map)` | remaps id's own child-axis (fill 0 → renormalize; zero rows → uniform) and every child's parent-axis (fill uniform for chance children, 0 for utility) |
| `set_kind` | rebuilds the table for the new kind; utility→ must have no children |

`map: &[Option<usize>]` in `remap_states`: `map[i] = Some(old)` means new
state i was old state `old`; `None` = brand-new state. The GUI's properties
dialog builds this from per-state `orig` tracking.

`experience` (per-row Dirichlet pseudo-counts) is **reset to None** by any
structural edit that changes the row space.

Cycle guard: `would_create_cycle(parent, child)` = `parent == child ||
reaches(child, parent)` via children-DFS. `add_edge` also rejects duplicate
edges and utility-node parents (utilities cannot have children).

`topo_order()` is Kahn's algorithm with roots sorted by name for
reproducibility (this keeps saved files and compiled VarIds stable).

## inference/ — the junction tree

### compile.rs

`CompiledNet::compile(net)`:
- variables = chance + decision nodes in topo order (utility nodes have NO
  VarId);
- `families[i]` = the CPT as a canonical Factor; decision nodes get a
  **uniform factor over themselves only** (their parents are informational);
- `family_vars[i]` = sorted family variable set (used for clique assignment
  and `family_posterior`).

`JunctionTree::build`:
1. **Moral graph**: marry chance-node families pairwise; decision nodes get
   only self–parent edges (their factor is over `[self]`, no marriage
   needed); **utility parents are married pairwise** (then utilities dropped)
   so EU queries hit a single clique.
2. **Min-fill triangulation** with min-weight (product of cardinalities)
   tie-break; records elimination cliques.
3. **Maximal cliques** by subset filtering.
4. **Max-weight spanning tree** (Kruskal over sepset sizes, union-find).
   Zero-weight edges are *included* to bridge disconnected components — the
   tree is always connected; empty sepsets act as scalar messages and this
   keeps the propagation code single-tree.
5. `home_of_family[v]` = smallest clique containing the family (CPT goes
   there); `belief_clique[v]` = smallest clique containing v.

### hugin.rs

`propagate(jt, pot, sep) -> Result<log_p_e, ConflictingEvidence>`:
- BFS-root the tree at clique 0; collect (leaves→root absorb), then the root
  sums to P(e). If `p_e <= 1e-300` → `ConflictingEvidence`.
- Normalize the root, then distribute (root→leaves). After that **every**
  clique potential is P(clique vars | e) — that's why showing all node
  beliefs at once is one propagation.
- `absorb(from, to, e)`: `new_sep = margin(pot[from] → sep vars)`;
  `pot[to] *= new_sep / old_sep` (0/0=0); `sep[e] = new_sep`.
- No mid-propagation renormalization: fine at editor scale; long chains with
  heavy evidence could underflow — see ROADMAP.md.

### mod.rs — Engine facade

```
Engine::compile(&Network)             // infallible; snapshots CPTs into clique potentials
engine.set_finding(node, Finding::Hard(i) | Likelihood(vec))   // marks Stale
engine.beliefs(node) / all_beliefs()  // propagates lazily if Stale
engine.log_prob_of_findings()
engine.family_posterior(node)         // P(node, parents | e) — one clique marginalization
engine.joint_posterior(&[nodes])      // clique fast path, else VE fallback over raw factors
engine.refresh_potentials(&net)       // same-structure CPT refresh (currently unused by GUI)
engine.set_evidence(Evidence) / evidence()  // save/restore pattern used by sensitivity & EU
```

State machine: `Stale → (propagate) → Ready | Conflict`. Any evidence change
marks Stale; propagation re-clones `init_pot` (cached CPT products), re-enters
all evidence (likelihood vectors multiplied into `belief_clique[var]`), and
propagates. **Structure or CPT edits invalidate the whole Engine** — the app
just recompiles (sub-ms; don't optimize prematurely).

Evidence entries for NodeIds no longer present in the net are silently
skipped (relevant after undo/redo).

Hard evidence and likelihood evidence are the same mechanism: a hard finding
is the indicator likelihood vector. Likelihood vectors need not be normalized.

## decision/

**single.rs** — `expected_utilities(engine, net, decision)`: for each action
d, enter `Hard(d)`, propagate, and sum over utility nodes
`Σ_pa P(pa|d,e) · U(pa)` via `joint_posterior(parents)` (single clique thanks
to utility-parent marriage). Returns `Vec<Option<f64>>` — `None` when the
action conflicts with evidence. Restores the engine's evidence before
returning. This is a *myopic* view for multiple decisions (others are
marginalized uniformly); exactness for sequences is ve_id's job.
`utility_expectation(engine, net, u)` is the per-utility-node E[U | e] used
for utility node display.

**ve_id.rs** — `solve_influence_diagram(net, evidence)`: textbook
(probability, utility)-potential variable elimination (Jensen & Nielsen):
- Decisions are ordered by topological order (this implements the total-order
  requirement; no-forgetting comes from the block structure below).
- Chance vars are partitioned into information blocks: `blocks[k]` = chance
  parents of decision k not yet placed; the tail block = everything else.
- Eliminate back-to-front: **sum** the tail, **max** the last decision,
  sum its block, … Sum rule: `p* = Σ Πp`, `u* = (Σ (Πp)(Σu)) / p*` (0/0=0).
  Max rule: maximize the summed utility potential; the probability part is
  constant over the decision in a well-formed diagram. `Factor::max_out`
  returns the argmax table = the policy.
- `Policy { decision, domain, domain_cards, best }` — `best` indexed by
  `encode_index` over the domain (sorted-VarId order).
- **Caveat (documented, accepted)**: evidence is folded in as likelihoods;
  if a finding sits downstream of a decision, the textbook max criterion can
  bias. Standard IDs (evidence-free solving, or evidence upstream) are exact.
  Tested against brute-force policy enumeration on the umbrella net.

## learn/

**cases.rs** — `CaseSet { nodes, rows: Vec<Vec<Option<usize>>>, weights }`.
CSV columns are matched to node names case-insensitively; `IDnum` skipped,
`NumCases`/`__count` = weight column; missing markers: empty, `*`, `?`, `NA`.
Cell values match state names or bare state indices. Unknown columns are
skipped silently; a file with zero matching columns errors.

**counting.rs** — per node with all family columns present: count complete
families; update `new_row = (ε·old_row + counts) / (ε + n)`, `new_ε = ε + n`
where ε is the row's experience (0 if absent or `use_experience=false`).

**em.rs** — the loop that justifies `family_posterior`:
1. dedupe identical evidence patterns (HashMap of rows → summed weight);
2. per iteration: `Engine::compile(net)` once; per unique case: set hard
   evidence for observed cells, accumulate `w · family_posterior(node)` into
   table-layout expected counts for **every** chance node (`fold_family`
   maps sorted-factor indices into `[parents..., self]` layout), accumulate
   `w · log P(e)`;
3. M-step: normalize `pseudo_count + expected counts` per row (zero rows →
   uniform);
4. stop when log-likelihood improves < `tol` or `max_iters`.
Cases with P(e)=0 under the current model are skipped and counted in
`EmReport::conflicting_cases`. The monotone log-likelihood invariant is
asserted in tests — if you touch EM and that test fails, EM is wrong, not
the test.

**structure/** — structure learning: rewires edges among a caller-chosen
set of existing chance nodes ("targets"); edges touching non-target nodes
are preserved and respected by cycle checks (`WorkDag` carries contracted
external-reachability arcs). All entry points take a `&SearchCtrl`
(progress callback + `AtomicBool` cancel flag) and mutate the network only
after the search fully succeeds, so `LearnError::Cancelled` never leaves a
half-edit. None fit parameters (run counting/EM after) except Structural EM.

- **stats.rs** `DataView`: column-major case view over targets; weighted
  contingency `counts(vars)` in last-axis-fastest layout, complete-case per
  query; tables over `1<<22` cells → `TableTooLarge`.
- **score.rs**: `CountSource` trait (observed counts from `DataView`, or
  expected counts from SEM's junction-tree provider) under a `FamilyScorer`
  that caches local scores by (child, sorted parents). Scores: BIC and BDeu
  (in-house `ln_gamma`/`chi2_sf` in math.rs — no stats dependency). BDeu
  score-equivalence of Markov-equivalent DAGs is asserted in tests.
- **hill_climb.rs**: greedy add/delete/reverse; decomposability means only
  touched families rescore. Plain ascent gets stuck in wrong-direction local
  optima even on sprinkler, so `random_restarts` defaults to 4 (seeded —
  same seed ⇒ identical output). `search()`/`climb()` are shared with GS
  (parent whitelist) and SEM (expected-count scorer).
- **ci.rs**: G² conditional-independence test with bnlearn-style df
  adjustment (zero-marginal rows/cols dropped per slice; df 0 ⇒ cannot
  reject, p = 1); `cmi()` shares the accumulation (G² = 2·N·CMI).
- **gs.rs**: Grow-Shrink Markov blankets (grow by descending pairwise MI,
  then shrink), used MMHC-style: blankets whitelist candidate parents for a
  hill climb. Symmetrization is OR-rule — AND loses true edges around
  near-deterministic nodes (asia's TbOrCa OR-gate: children are independent
  of it given its parents, a faithfulness violation).
- **pc.rs**: PC-stable (per-level neighborhood snapshots ⇒ order-independent
  skeleton), v-structures (conflicts counted, first-come kept), Meek rules
  R1–R3 (R4 needs background knowledge we don't have), then a Dor–Tarsi
  consistent extension picks the DAG that is written into the network.
  `PcReport::cpdag` keeps directed vs undirected edges so the GUI can flag
  arbitrarily-oriented ones; `forced_orientations > 0` signals inconsistent
  CI answers.
- **nb_tan.rs**: Naive Bayes (class → every feature) and TAN (Chow-Liu:
  Kruskal max-spanning-tree over pairwise CMI given the class, directed
  away from the root, class parent of every feature).
- **sem.rs**: Structural EM (Friedman 1998). Outer loop: parametric EM on a
  working clone → `PosteriorCounts` (rows complete over the targets are
  counted directly; incomplete patterns deduped, then
  `Engine::joint_posterior` over each candidate family, folded into table
  layout; memoized per outer iteration) → hill climb on the expected score →
  stop on unchanged structure or `Δscore < tol`. Effective N is identical
  for every candidate family (total non-conflicting weight), so there is no
  available-case bias. The expected-score trace is non-decreasing — asserted
  in tests, same contract as EM's log-likelihood.

Recovery tests (tests/structure_tests.rs) forward-sample sprinkler/asia and
compare learned vs true structure up to Markov equivalence via
`dag_to_cpdag` (skeleton + v-structures + Meek closure).

## sample/

- `forward_sample`: topo walk; chance nodes drawn from CPT row given sampled
  parents; decision nodes uniform; utilities skipped.
- `generate_cases(net, n, missing_rate, rng)`: generates a synthetic case file.
- `lw_beliefs(net, evidence, n, rng)`: likelihood weighting; hard findings
  fix the state and multiply weight by P(state|pa); likelihood findings
  sample then weight by lik[state]. Total weight 0 → ConflictingEvidence.
- All functions take `&mut impl Rng`; tests use `StdRng::seed_from_u64`.
  **rand 0.10**: `random`/`random_range` live on the `RngExt` trait —
  import `rand::{Rng, RngExt}`.

## sensitivity.rs

`sensitivity_to_findings(engine, net, target, candidates)`: for each
candidate F, for each state f with P(f|e)>0: temporarily enter `Hard(f)`,
propagate, read P(T|f,e); accumulate
`MI = Σ_f P(f|e) Σ_t P(t|f,e) log2(P(t|f,e)/P(t|e))`, entropy-reduction %
(= 100·MI/H(T|e)), and variance reduction when all target states have numeric
`State::value`s. Saves and restores the engine's evidence. Cost = Σ|states|
propagations — fine at editor scale; the per-candidate loop is the natural
rayon point later.

## error.rs

One thiserror enum per concern: `ModelError` (structural edit rejections —
the GUI surfaces these verbatim), `InferenceError` (`ConflictingEvidence` is
the load-bearing variant: **P(e)=0 must be a typed error, never NaN**),
`IoError`, `CaseError`, `LearnError`, `IdError`.
