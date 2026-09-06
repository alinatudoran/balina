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
    bridge.mark(doc.delete_items(&[a], &[]));
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
    ops::edit::delete_items(&mut s, &[id], &[]).unwrap();
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

    // Simulate a small case file to learn from.
    let tmp = std::env::temp_dir().join("balina_session_test_cases.csv");
    let msg = ops::learn::simulate_cases_to_file(&s.doc.net, tmp.clone(), 2_000, 0.0).unwrap();
    assert!(msg.contains("2000 cases"));

    let opts = StructureLearnOpts {
        path: tmp.clone(),
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
    let outcome = run_structure_job(input, &jc);
    let result = apply_structure_outcome(&mut s, outcome, started_seq).unwrap();
    assert!(s.job_cancel.is_none(), "busy slot cleared");
    assert!(s.bridge.compiled && !s.bridge.conflict);
    assert!(result.report.contains("Links:"));
    // Staleness: edit mid-job → result discarded.
    let input = prepare_structure_job(&mut s, opts).unwrap();
    let started_seq = input.started_seq;
    let outcome = run_structure_job(input, &jc);
    s.doc.begin_change(); // concurrent edit
    assert!(matches!(
        apply_structure_outcome(&mut s, outcome, started_seq),
        Err(CmdError::Stale(_))
    ));
    let _ = std::fs::remove_file(tmp);
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
