# File Formats

All formats exchange `bn_core::io::Document { network, visual }` — visual
metadata lives in bn-core (not the app) precisely so positions can round-trip
through XDSL. Each format has its own DTO layer; **runtime types never derive
Serialize directly**, so the model can evolve without breaking files.

Dispatch: `io::load(path)` / `io::save(doc, path)` pick the format from the
extension (`Format::from_path`): `.balina`/`.json` → native, `.xml`/`.bif`/
`.xmlbif` → XMLBIF, `.xdsl` → XDSL. `load_str`/`save_str` exist for tests.
`save` returns `Vec<Warning>` — currently only `Warning::Lossy(String)`,
which the app prints to the message log.

**Shared value ordering**: all three formats list table values with parents
as outer axes (declared order) and the node itself varying fastest — exactly
bn-core's memory layout, so table data is copied verbatim. If you add a
format, verify its ordering convention first; this is the classic
interchange bug.

## Native `.balina` (io/native.rs)

Versioned JSON:

```json
{
  "format": "balina-net", "version": 1,
  "name": "Chest Clinic (Asia)", "comment": "",
  "nodes": [
    { "name": "VisitAsia", "title": "", "kind": "Chance",
      "states": [{"name": "yes", "value": null}, {"name": "no", "value": null}],
      "parents": [],            // parent NAMES, in table-axis order
      "table": [0.01, 0.99],    // flat, layout as above
      "experience": null, "comment": "" },
    ...
  ],
  "visual": { "VisitAsia": {"x": 40.0, "y": 40.0, "display": "BeliefBars", "color": null}, ... }
}
```

- Nodes are written in **topological order** so loading is two passes:
  create all nodes, then add edges (in stored parent order — `add_edge`
  appends parents, so order is preserved) and set tables. Note `add_edge`
  reshapes tables, which is why tables are set **after** edges.
- Everything non-essential is `#[serde(default)]` — forward compatible.
- Visual is keyed by node **name** (slotmap keys are meaningless across
  sessions). Rename-then-save is fine because saving re-derives the map from
  live NodeIds.
- Not yet persisted: evidence, camera, net-level window state (ROADMAP).

## XMLBIF v0.3 (io/xmlbif.rs)

- Chance-only format. Decision nodes are written as chance nodes with a
  uniform TABLE; utility nodes are **skipped**; both emit `Warning::Lossy`.
- `<VARIABLE>` holds `<NAME>`, `<OUTCOME>`s, and layout as
  `<PROPERTY>position = (x, y)</PROPERTY>` (JavaBayes convention).
- `<DEFINITION>` holds `<FOR>`, `<GIVEN>`s (= parent order), `<TABLE>`
  (whitespace-separated floats).
- Parser is quick-xml **manual events** (no serde feature — XMLBIF's mixed
  content fights serde). It tracks an element path stack and two builder
  structs; `<PROBABILITY>` is accepted as an alias for `<DEFINITION>`.
  Writing is plain string building with an `esc()` helper.

## XDSL / GeNIe (io/xdsl.rs)

- Full influence-diagram support: `<cpt>`, `<decision>`, `<utility>`
  (+ `<deterministic>` parsed as cpt) inside `<nodes>`; each has
  `<state id=…/>`s, `<parents>` (space-separated ids), and
  `<probabilities>`/`<utilities>`.
- Layout and titles live in `<extensions><genie><node id=…><name>` and
  `<position>x1 y1 x2 y2</position>` (we store x1 y1 and write x1+120, y1+60
  as the second corner).
- XDSL ids must be identifier-like: `ident()` sanitizes names on write
  (non-alphanumerics → `_`, leading digit prefixed). Loading a file we wrote
  therefore may have different names than the in-memory net it came from —
  round-trip tests compare *semantics* (beliefs/MEU), not names.
- Net name comes from the `<smile id=…>` attribute.

## Case files (learn/cases.rs — CSV, not a network format)

Header row of node names (case-insensitive match); `IDnum` column ignored;
`NumCases`/`__count` column = case weight; missing markers: empty, `*`, `?`,
`NA`. Values are state names or bare indices. `write_cases` emits state
names and adds `NumCases` only when weights are non-uniform.

## Round-trip guarantees (enforced in tests/feature_tests.rs)

- native: `save(load(save(x)))` byte-stable; beliefs equal to 1e-12.
- XMLBIF: beliefs equal to 1e-12 (chance-only nets).
- XDSL: MEU and optimal policies equal on influence diagrams.
Float noise ~1e-16 after reload is expected (different summation order from a
different topo order) — compare with tolerance, never exact equality.
