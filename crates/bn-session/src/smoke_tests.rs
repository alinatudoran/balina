//! Headless smoke tests for the session/engine plumbing — the same code
//! paths the UI drives (ported from the egui app, plus op-layer tests
//! against `ops`).

use std::path::Path;

use bn_core::model::NodeKind;

use crate::doc::{Dirt, Document, Point};
use crate::engine_bridge::EngineBridge;
use crate::error::CmdError;
use crate::ops;
use crate::session::Session;
use crate::views;

fn examples_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn load_asia() -> Document {
    let iodoc = bn_core::io::load(&examples_dir().join("asia.balina"))
        .expect("examples/asia.balina must exist");
    Document::from_io_document(iodoc, None)
}

fn sync(bridge: &mut EngineBridge, doc: &Document) {
    if doc.auto_update && bridge.is_dirty() && !doc.net.is_empty() {
        bridge.recompute(doc);
    }
}

#[test]
fn open_compile_evidence_undo_cycle() {
    let mut doc = load_asia();
    let mut bridge = EngineBridge::default();
    bridge.mark(Dirt::Structure);
    sync(&mut bridge, &doc);
    assert!(bridge.compiled);
    assert!(!bridge.conflict);

    let dysp = doc.net.find_by_name("Dyspnea").unwrap();
    let tub = doc.net.find_by_name("Tuberculosis").unwrap();
    let prior_tub = bridge.beliefs[tub][0];

    // Click a state row → finding → recompute.
    let dirt = doc.toggle_finding(dysp, 0);
    bridge.mark(dirt);
    sync(&mut bridge, &doc);
    let post_tub = bridge.beliefs[tub][0];
    assert!(post_tub > prior_tub, "dyspnea should raise P(tuberculosis)");
    assert!(bridge.log_p_e.is_some());

    // Click again → retracts.
    let dirt = doc.toggle_finding(dysp, 0);
    bridge.mark(dirt);
    sync(&mut bridge, &doc);
    assert!((bridge.beliefs[tub][0] - prior_tub).abs() < 1e-12);

    // Undo both evidence changes.
    assert!(doc.can_undo());
    bridge.mark(doc.undo());
    bridge.mark(doc.undo());
    sync(&mut bridge, &doc);
    assert!(doc.evidence.is_empty());
}

#[test]
fn add_link_delete_undo_roundtrip() {
    let mut doc = Document::new();
    let mut bridge = EngineBridge::default();
    let a = doc.add_node_at(NodeKind::Chance, Point::new(10.0, 10.0)).unwrap();
    let b = doc.add_node_at(NodeKind::Chance, Point::new(200.0, 10.0)).unwrap();
    doc.begin_change();
    doc.net.add_edge(a, b).unwrap();
    bridge.mark(Dirt::Structure);
    sync(&mut bridge, &doc);
    assert_eq!(bridge.beliefs.len(), 2);

    // Cycle must be rejected by the canvas guard.
    assert!(doc.net.would_create_cycle(b, a));

    // Delete node A; B's table must contract; undo restores everything.
    bridge.mark(doc.delete_items(&[a], &[], &[]));
    sync(&mut bridge, &doc);
    assert_eq!(doc.net.len(), 1);
    bridge.mark(doc.undo());
    sync(&mut bridge, &doc);
    assert_eq!(doc.net.len(), 2);
    assert_eq!(doc.net.edges().len(), 1);
    assert!(!bridge.conflict);
}

#[test]
fn decision_network_shows_expected_utilities() {
    let iodoc = bn_core::io::load(&examples_dir().join("umbrella.balina"))
        .expect("examples/umbrella.balina must exist");
    let doc = Document::from_io_document(iodoc, None);
    let mut bridge = EngineBridge::default();
    bridge.mark(Dirt::Structure);
    sync(&mut bridge, &doc);
    let take = doc.net.find_by_name("Umbrella").unwrap();
    let util = doc.net.find_by_name("Satisfaction").unwrap();
    let eu = &bridge.decision_eu[take];
    assert_eq!(eu.len(), 2);
    assert!(eu[0].unwrap() > 0.0 && eu[1].unwrap() > 0.0);
    assert!(bridge.utility_ev.contains_key(util));
}

#[test]
fn structure_learning_clone_swap_undo() {
    use bn_core::learn::structure::{self, HillClimbOptions, SearchCtrl};
    use bn_core::learn::{learn_counting, CountingOptions};
    use rand::SeedableRng;

    let mut doc = load_asia();
    let mut bridge = EngineBridge::default();
    bridge.mark(Dirt::Structure);
    sync(&mut bridge, &doc);
    let before_edges = doc.net.edges();
    let seq0 = doc.change_seq;

    // The worker-thread body, run synchronously: clone → learn → fit.
    let mut clone = doc.net.clone();
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let cases = bn_core::sample::generate_cases(&clone, 5_000, 0.0, &mut rng);
    let targets = structure::default_targets(&clone);
    structure::learn_hill_climb(
        &mut clone,
        &cases,
        &targets,
        &HillClimbOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    learn_counting(&mut clone, &cases, &CountingOptions::default()).unwrap();

    // Apply on the session side: one undoable step, swap the net.
    assert_eq!(doc.change_seq, seq0, "no edits while 'worker' ran");
    doc.begin_change();
    doc.net = clone;
    doc.ensure_visuals();
    bridge.mark(Dirt::Structure);
    sync(&mut bridge, &doc);
    assert!(bridge.compiled && !bridge.conflict);
    // Every node still has a visual (NodeIds survive the clone-swap).
    for id in doc.net.node_ids() {
        assert!(doc.visual.contains_key(id), "visual lost after swap");
    }
    // One undo restores the original structure.
    bridge.mark(doc.undo());
    sync(&mut bridge, &doc);
    assert_eq!(doc.net.edges(), before_edges);
    assert!(bridge.compiled && !bridge.conflict);
}

#[test]
fn change_seq_semantics() {
    let mut doc = Document::new();
    let s0 = doc.change_seq;
    doc.begin_change();
    assert_eq!(doc.change_seq, s0 + 1, "begin_change bumps");
    doc.begin_visual_change();
    assert_eq!(doc.change_seq, s0 + 1, "visual change does not bump");
    doc.undo();
    assert_eq!(doc.change_seq, s0 + 2, "undo bumps");
    doc.redo();
    assert_eq!(doc.change_seq, s0 + 3, "redo bumps");
}

// ---------------------------------------------------------------------------
// Op-layer tests (drive ops::* against a Session, no UI runtime)
// ---------------------------------------------------------------------------

#[test]
fn op_flow_open_evidence_undo() {
    let mut s = Session::new();
    ops::file::doc_open(&mut s, examples_dir().join("asia.balina")).unwrap();
    assert_eq!(s.doc.net.len(), 8);
    assert!(s.bridge.compiled && !s.bridge.conflict);

    let dysp = s.doc.net.find_by_name("Dyspnea").unwrap();
    let tub = s.doc.net.find_by_name("Tuberculosis").unwrap();
    let tub_prior = s.bridge.beliefs[tub][0];

    ops::evidence::toggle_finding(&mut s, dysp, 0).unwrap();
    assert_eq!(s.doc.evidence.len(), 1);
    assert!(s.bridge.beliefs[tub][0] > tub_prior);
    assert!(s.bridge.log_p_e.is_some());

    ops::edit::undo(&mut s);
    assert_eq!(s.doc.evidence.len(), 0);
    assert!(s.doc.can_redo());
}

#[test]
fn stale_node_ids_become_bad_request() {
    let mut s = Session::new();
    let id = ops::edit::add_node(&mut s, NodeKind::Chance, 10.0, 20.0).unwrap();
    assert!(views::check_node(&s.doc.net, id).is_ok());
    // Delete the node; the retained id must fail cleanly, never panic.
    ops::edit::delete_items(&mut s, &[id], &[], &[]).unwrap();
    assert!(s.doc.net.is_empty());
    assert!(matches!(
        ops::evidence::toggle_finding(&mut s, id, 0),
        Err(CmdError::BadRequest(_))
    ));
    assert!(matches!(views::check_node(&s.doc.net, id), Err(CmdError::BadRequest(_))));
    // Undo revives the node and its id becomes valid again.
    ops::edit::undo(&mut s);
    assert!(views::check_node(&s.doc.net, id).is_ok());
}

#[test]
fn set_cpt_shape_error_rolls_back() {
    let mut s = Session::new();
    let id = ops::edit::add_node(&mut s, NodeKind::Chance, 0.0, 0.0).unwrap();
    let before = s.doc.net.node(id).table.data.clone();
    // Wrong length → error, and the failed begin_change snapshot is rolled back.
    let err = ops::cpt::set_cpt(&mut s, id, vec![0.5; 5]).unwrap_err();
    assert!(matches!(err, CmdError::Model(_)));
    assert_eq!(
        s.doc.net.node(id).table.data,
        before,
        "failed set_cpt must leave the table unchanged"
    );
    // A correct edit works and marks Params dirt (beliefs update).
    ops::cpt::set_cpt(&mut s, id, vec![0.3, 0.7]).unwrap();
    assert!((s.bridge.beliefs[id][0] - 0.3).abs() < 1e-9);
}

#[test]
fn cpt_view_row_labels() {
    let mut s = Session::new();
    ops::file::doc_open(&mut s, examples_dir().join("asia.balina")).unwrap();
    let dysp = s.doc.net.find_by_name("Dyspnea").unwrap();
    let v = ops::cpt::get_cpt(&s, dysp).unwrap();
    assert_eq!(v.node, dysp);
    assert!(!v.is_utility);
    assert_eq!(v.parent_headers.len(), s.doc.net.node(dysp).parents.len());
    for row in &v.rows {
        assert_eq!(row.labels.len(), v.parent_headers.len());
        assert_eq!(row.values.len(), v.out_card);
    }
}

#[test]
fn structure_job_prepare_run_apply() {
    use crate::jobs::JobCtx;
    use crate::ops::learn::{
        apply_structure_outcome, prepare_structure_job, run_structure_job, LearnMethod,
        ScoreChoice, StructAlgo, StructureLearnOpts,
    };
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut s = Session::new();
    ops::file::doc_open(&mut s, examples_dir().join("asia.balina")).unwrap();

    // Simulate a small case set to learn from.
    let cases = ops::learn::simulate_cases_csv(&s.doc.net, 2_000, 0.0).unwrap().into_bytes();

    let opts = StructureLearnOpts {
        algo: StructAlgo::HillClimb,
        score: ScoreChoice::Bic,
        ess: 1.0,
        max_parents: 4,
        alpha: 0.05,
        class_node: None,
        param_method: LearnMethod::Counting,
        em_iters: 50,
        required_edges: vec![],
        forbidden_edges: vec![],
    };
    let input = prepare_structure_job(&mut s, opts.clone()).unwrap();
    assert!(s.job_cancel.is_some(), "busy while job runs");
    assert!(matches!(prepare_structure_job(&mut s, opts.clone()), Err(CmdError::Busy(_))));
    let started_seq = input.started_seq;
    let jc = JobCtx::new(Arc::new(AtomicBool::new(false)), |_| {});
    let outcome = run_structure_job(input, &cases, None, &jc);
    let result = apply_structure_outcome(&mut s, outcome, started_seq).unwrap();
    assert!(s.job_cancel.is_none(), "busy slot cleared");
    assert!(s.bridge.compiled && !s.bridge.conflict);
    assert!(result.report.contains("Links:"));
    // Staleness: edit mid-job → result discarded.
    let input = prepare_structure_job(&mut s, opts).unwrap();
    let started_seq = input.started_seq;
    let outcome = run_structure_job(input, &cases, None, &jc);
    s.doc.begin_change(); // concurrent edit
    assert!(matches!(
        apply_structure_outcome(&mut s, outcome, started_seq),
        Err(CmdError::Stale(_))
    ));
}

/// The by-name StructurePatch path (web worker) must produce exactly the same
/// network as the clone-swap path (desktop).
#[test]
fn structure_patch_equivalent_to_outcome_swap() {
    use crate::jobs::JobCtx;
    use crate::ops::learn::{
        apply_structure_outcome, prepare_structure_job, run_structure_job, LearnMethod,
        ScoreChoice, StructAlgo, StructureLearnOpts, StructureOutcome,
    };
    use crate::patch::{apply_structure_patch, structure_patch};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut s_swap = Session::new();
    ops::file::doc_open(&mut s_swap, examples_dir().join("asia.balina")).unwrap();
    let mut s_patch = Session::new();
    ops::file::doc_open(&mut s_patch, examples_dir().join("asia.balina")).unwrap();

    let cases = ops::learn::simulate_cases_csv(&s_swap.doc.net, 2_000, 0.0).unwrap().into_bytes();
    let opts = StructureLearnOpts {
        algo: StructAlgo::HillClimb,
        score: ScoreChoice::Bic,
        ess: 1.0,
        max_parents: 4,
        alpha: 0.05,
        class_node: None,
        param_method: LearnMethod::Counting,
        em_iters: 50,
        required_edges: vec![],
        forbidden_edges: vec![],
    };
    let jc = JobCtx::new(Arc::new(AtomicBool::new(false)), |_| {});

    // One job result, applied both ways.
    let input = prepare_structure_job(&mut s_swap, opts).unwrap();
    let started_seq = input.started_seq;
    let before = input.net.clone();
    let outcome = run_structure_job(input, &cases, None, &jc);
    let StructureOutcome::Done { net: learned, report, summary, warnings } = outcome else {
        panic!("job did not finish");
    };
    let patch = structure_patch(&before, &learned);
    assert!(!patch.nodes.is_empty(), "learning should change something");
    // Round-trip the patch through JSON like the worker protocol does.
    let patch: crate::patch::StructurePatch =
        serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();

    s_patch.job_cancel = Some(Arc::new(AtomicBool::new(false))); // as if a job ran
    let patch_seq = s_patch.doc.change_seq;
    apply_structure_patch(
        &mut s_patch,
        &patch,
        report.clone(),
        summary.clone(),
        warnings.clone(),
        patch_seq,
    )
    .unwrap();
    assert!(s_patch.job_cancel.is_none());
    apply_structure_outcome(
        &mut s_swap,
        StructureOutcome::Done { net: learned, report, summary, warnings },
        started_seq,
    )
    .unwrap();

    // Same edges (by name) and same tables/experience per node.
    let name_edges = |net: &bn_core::model::Network| {
        let mut e: Vec<(String, String)> = net
            .edges()
            .into_iter()
            .map(|(p, c)| (net.node(p).name.clone(), net.node(c).name.clone()))
            .collect();
        e.sort();
        e
    };
    assert_eq!(name_edges(&s_swap.doc.net), name_edges(&s_patch.doc.net));
    for (_, n_swap) in s_swap.doc.net.nodes() {
        let id = s_patch.doc.net.find_by_name(&n_swap.name).unwrap();
        let n_patch = s_patch.doc.net.node(id);
        let parents_swap: Vec<&str> =
            n_swap.parents.iter().map(|&p| s_swap.doc.net.node(p).name.as_str()).collect();
        let parents_patch: Vec<&str> =
            n_patch.parents.iter().map(|&p| s_patch.doc.net.node(p).name.as_str()).collect();
        assert_eq!(parents_swap, parents_patch, "parent order for {}", n_swap.name);
        assert_eq!(n_swap.table.data.len(), n_patch.table.data.len(), "table {}", n_swap.name);
        for (a, b) in n_swap.table.data.iter().zip(&n_patch.table.data) {
            assert!((a - b).abs() < 1e-12, "table values for {}", n_swap.name);
        }
        assert_eq!(n_swap.experience, n_patch.experience, "experience for {}", n_swap.name);
    }
}

#[test]
fn id_solution_sensitivity_and_ancestors() {
    let mut s = Session::new();
    ops::file::doc_open(&mut s, examples_dir().join("umbrella.balina")).unwrap();
    // ID solver produces a policy text.
    let sol = ops::tools::solve_influence_diagram(&s).unwrap();
    assert!(sol.text.contains("Maximum expected utility"));
    // Sensitivity runs on a chance target.
    let weather = s.doc.net.find_by_name("Weather").unwrap();
    let rows = ops::tools::run_sensitivity(&mut s, weather).unwrap();
    assert!(!rows.is_empty());
    // Ancestor sets support the canvas cycle checks.
    let anc = views::ancestor_sets(&s.doc.net);
    let sat = s.doc.net.find_by_name("Satisfaction").unwrap();
    assert!(anc[sat].contains(&weather), "Weather is an ancestor of Satisfaction");
    assert!(anc[weather].is_empty());
}

// ---- sticky notes ----------------------------------------------------------

#[test]
fn note_ops_undo_and_stale_ids() {
    let mut s = Session::new();
    let seq0 = s.doc.change_seq;

    // Add + edit: visual-only, so change_seq never moves.
    let id = ops::edit::add_note(&mut s, 30.0, 40.0).unwrap();
    ops::edit::set_note_text(&mut s, id, "hello".into()).unwrap();
    ops::edit::set_note_color(&mut s, id, [181, 220, 255]).unwrap();
    ops::edit::move_items(&mut s, &[], &[(id, 5.0, 6.0)]).unwrap();
    assert_eq!(s.doc.change_seq, seq0);
    assert_eq!(s.doc.notes[id].text, "hello");
    assert_eq!(s.doc.notes[id].pos, Point::new(5.0, 6.0));

    // Clamps.
    ops::edit::resize_note(&mut s, id, 1.0, 1.0).unwrap();
    assert_eq!((s.doc.notes[id].w, s.doc.notes[id].h), crate::doc::NOTE_MIN_SIZE);
    ops::edit::nudge_note_font(&mut s, id, 1000.0).unwrap();
    assert_eq!(s.doc.notes[id].font_size, crate::doc::NOTE_FONT_RANGE.1);

    // Collapse keeps the expanded size; still no change_seq movement.
    ops::edit::set_note_collapsed(&mut s, id, true).unwrap();
    assert!(s.doc.notes[id].collapsed);
    assert_eq!((s.doc.notes[id].w, s.doc.notes[id].h), crate::doc::NOTE_MIN_SIZE);
    assert_eq!(s.doc.change_seq, seq0);
    // Setting the same state is a no-op: one undo reverts the real toggle.
    ops::edit::set_note_collapsed(&mut s, id, true).unwrap();
    ops::edit::undo(&mut s);
    assert!(!s.doc.notes[id].collapsed);
    ops::edit::redo(&mut s);
    assert!(s.doc.notes[id].collapsed);
    ops::edit::set_note_collapsed(&mut s, id, false).unwrap();
    // Undo/redo themselves bump change_seq (any undo may touch the model) —
    // re-baseline before the notes-only-delete check below.
    let seq0 = s.doc.change_seq;

    // Notes-only delete: no change_seq bump; undo restores the note.
    ops::edit::delete_items(&mut s, &[], &[], &[id]).unwrap();
    assert_eq!(s.doc.change_seq, seq0);
    assert!(s.doc.notes.is_empty());
    assert!(matches!(
        ops::edit::set_note_text(&mut s, id, "x".into()),
        Err(CmdError::BadRequest(_))
    ));
    ops::edit::undo(&mut s);
    assert_eq!(s.doc.notes[id].text, "hello");
}

#[test]
fn unchanged_note_text_is_not_an_undo_step() {
    let mut s = Session::new();
    let id = ops::edit::add_note(&mut s, 0.0, 0.0).unwrap();
    ops::edit::set_note_text(&mut s, id, "hello".into()).unwrap();
    ops::edit::set_note_text(&mut s, id, "hello".into()).unwrap(); // no-op
    // One undo must revert the ORIGINAL text edit, not a phantom no-op step.
    ops::edit::undo(&mut s);
    assert_eq!(s.doc.notes[id].text, "");
}

#[test]
fn notes_roundtrip_through_io_document() {
    let mut s = Session::new();
    let id = ops::edit::add_note(&mut s, 12.0, 34.0).unwrap();
    ops::edit::set_note_text(&mut s, id, "line1\nline2".into()).unwrap();
    ops::edit::resize_note(&mut s, id, 240.0, 130.0).unwrap();
    ops::edit::nudge_note_font(&mut s, id, 4.0).unwrap();
    ops::edit::set_note_collapsed(&mut s, id, true).unwrap();

    let iodoc = s.doc.to_io_document();
    let back = Document::from_io_document(iodoc, None);
    assert_eq!(back.notes.len(), 1);
    let n = back.notes.values().next().unwrap();
    assert_eq!(*n, s.doc.notes[id]);
}
