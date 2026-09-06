//! Structure learning: recover known networks from forward-sampled data.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use bn_core::learn::structure::{
    self, dag::dag_to_cpdag, HillClimbOptions, ScoreKind, SearchCtrl,
};
use bn_core::learn::{learn_counting, CountingOptions};
use bn_core::model::{Network, NodeId};
use bn_core::sample::generate_cases;
use bn_core::{Engine, Evidence};
use rand::rngs::StdRng;
use rand::SeedableRng;

use common::{asia, sprinkler};

/// Edges of `net` restricted to `targets`, as index pairs into `targets`.
fn edge_indices(net: &Network, targets: &[NodeId]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = net
        .edges()
        .into_iter()
        .filter_map(|(p, c)| {
            let pi = targets.iter().position(|&t| t == p)?;
            let ci = targets.iter().position(|&t| t == c)?;
            Some((pi as u32, ci as u32))
        })
        .collect();
    out.sort_unstable();
    out
}

/// Undirected skeleton as (min, max) pairs.
fn skeleton(edges: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> =
        edges.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn hamming(a: &[(u32, u32)], b: &[(u32, u32)]) -> usize {
    a.iter().filter(|e| !b.contains(e)).count() + b.iter().filter(|e| !a.contains(e)).count()
}

/// Clear all edges among targets and reset CPTs to uniform priors, so the
/// learner starts from an uninformed network.
fn blank_structure(net: &mut Network, targets: &[NodeId]) {
    for (p, c) in net.edges() {
        if targets.contains(&p) && targets.contains(&c) {
            net.remove_edge(p, c).unwrap();
        }
    }
    for &id in targets {
        let len = net.table_len(id);
        let out = net.node(id).out_card();
        net.set_table(id, bn_core::model::Table { data: vec![1.0 / out as f64; len] })
            .unwrap();
    }
}

#[test]
fn hill_climb_recovers_sprinkler_cpdag() {
    let (truth, ids) = sprinkler();
    let mut rng = StdRng::seed_from_u64(7);
    let cases = generate_cases(&truth, 20_000, 0.0, &mut rng);
    let true_cpdag = dag_to_cpdag(&edge_indices(&truth, &ids), ids.len());

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    let report = structure::learn_hill_climb(
        &mut net,
        &cases,
        &ids,
        &HillClimbOptions { max_parents: 3, ..Default::default() },
        &SearchCtrl::default(),
    )
    .unwrap();
    let learned = dag_to_cpdag(&edge_indices(&net, &ids), ids.len());
    assert_eq!(learned, true_cpdag, "learned CPDAG differs (score {})", report.score);
    // The trace resets between restarts, but the reported best score is the
    // trace maximum and each climb only moves uphill.
    assert_eq!(report.score, report.score_trace.iter().copied().fold(f64::MIN, f64::max));
}

#[test]
fn hill_climb_bdeu_close_to_asia_skeleton() {
    let truth = asia();
    let ids = structure::default_targets(&truth);
    let mut rng = StdRng::seed_from_u64(11);
    let cases = generate_cases(&truth, 50_000, 0.0, &mut rng);
    let true_skel = skeleton(&edge_indices(&truth, &ids));

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    structure::learn_hill_climb(
        &mut net,
        &cases,
        &ids,
        &HillClimbOptions { score: ScoreKind::Bdeu { ess: 10.0 }, ..Default::default() },
        &SearchCtrl::default(),
    )
    .unwrap();
    let learned_skel = skeleton(&edge_indices(&net, &ids));
    // The VisitAsia→Tuberculosis link is very weak (0.05 vs 0.01); allow it
    // to be missed but nothing else.
    assert!(
        hamming(&learned_skel, &true_skel) <= 1,
        "skeleton too far from truth: {learned_skel:?} vs {true_skel:?}"
    );
}

#[test]
fn hill_climb_deterministic_and_cancellable() {
    let (truth, ids) = sprinkler();
    let mut rng = StdRng::seed_from_u64(3);
    let cases = generate_cases(&truth, 5_000, 0.0, &mut rng);
    let opts = HillClimbOptions { random_restarts: 2, seed: 42, ..Default::default() };

    let run = || {
        let mut net = truth.clone();
        blank_structure(&mut net, &ids);
        structure::learn_hill_climb(&mut net, &cases, &ids, &opts, &SearchCtrl::default())
            .unwrap()
            .edges
    };
    assert_eq!(run(), run(), "same seed must give identical edges");

    // Cancel from the first progress tick: net must be left unmodified.
    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    let before = net.edges();
    let cancel = AtomicBool::new(false);
    let flag = &cancel;
    let progress = move |_p: structure::Progress| flag.store(true, Ordering::Relaxed);
    let err = structure::learn_hill_climb(
        &mut net,
        &cases,
        &ids,
        &opts,
        &SearchCtrl { progress: Some(&progress), cancel: Some(&cancel) },
    )
    .unwrap_err();
    assert!(matches!(err, bn_core::LearnError::Cancelled));
    assert_eq!(net.edges(), before, "cancel must not modify the network");
}

#[test]
fn hill_climb_then_counting_recovers_beliefs() {
    let (truth, ids) = sprinkler();
    let mut rng = StdRng::seed_from_u64(5);
    let cases = generate_cases(&truth, 20_000, 0.0, &mut rng);

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    structure::learn_hill_climb(
        &mut net,
        &cases,
        &ids,
        &HillClimbOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    learn_counting(&mut net, &cases, &CountingOptions::default()).unwrap();

    let mut e_true = Engine::compile(&truth);
    let mut e_learn = Engine::compile(&net);
    e_true.set_evidence(Evidence::new());
    e_learn.set_evidence(Evidence::new());
    for &id in &ids {
        let a = e_true.beliefs(id).unwrap();
        let b = e_learn.beliefs(id).unwrap();
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 0.02, "belief mismatch on {}: {a:?} vs {b:?}",
                truth.node(id).name);
        }
    }
}

#[test]
fn pc_stable_recovers_sprinkler() {
    let (truth, ids) = sprinkler();
    let mut rng = StdRng::seed_from_u64(9);
    let cases = generate_cases(&truth, 20_000, 0.0, &mut rng);

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    let report = structure::learn_pc(
        &mut net,
        &cases,
        &ids,
        &structure::PcOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    // Exact skeleton.
    let learned_skel = skeleton(&edge_indices(&net, &ids));
    let true_skel = skeleton(&edge_indices(&truth, &ids));
    assert_eq!(learned_skel, true_skel);
    // The Sprinkler→WetGrass←Rain collider is compelled; the Cloudy edges
    // are reversible.
    let [c, s, r, w] = ids;
    assert!(report.cpdag.directed.contains(&(s, w)));
    assert!(report.cpdag.directed.contains(&(r, w)));
    let und: Vec<_> = report.cpdag.undirected.iter().collect();
    assert_eq!(und.len(), 2, "expected 2 reversible edges: {und:?}");
    for &(a, b) in &report.cpdag.undirected {
        assert!(a == c || b == c, "reversible edges touch Cloudy");
    }
    assert_eq!(report.forced_orientations, 0);
    assert_eq!(report.v_structure_conflicts, 0);
    // The applied DAG is a consistent extension: same CPDAG as the truth.
    let learned_cpdag = dag_to_cpdag(&edge_indices(&net, &ids), ids.len());
    let true_cpdag = dag_to_cpdag(&edge_indices(&truth, &ids), ids.len());
    assert_eq!(learned_cpdag, true_cpdag);
}

#[test]
fn gs_blankets_and_hybrid_on_asia() {
    let truth = asia();
    let ids = structure::default_targets(&truth);
    let mut rng = StdRng::seed_from_u64(13);
    let cases = generate_cases(&truth, 50_000, 0.0, &mut rng);

    let by_name = |n: &str| truth.find_by_name(n).unwrap();
    let blankets = structure::grow_shrink_blankets(
        &truth,
        &cases,
        &ids,
        &structure::CiOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    let blanket_of = |n: &str| -> Vec<String> {
        let (_, b) = blankets.iter().find(|(id, _)| *id == by_name(n)).unwrap();
        let mut names: Vec<String> =
            b.iter().map(|&id| truth.node(id).name.clone()).collect();
        names.sort();
        names
    };
    // MB(Smoking) = its children {LungCancer, Bronchitis} (no spouses:
    // LungCancer's other parent set is empty… but TbOrCa's parents make
    // Tuberculosis a spouse of LungCancer, not of Smoking).
    assert_eq!(blanket_of("Smoking"), vec!["Bronchitis", "LungCancer"]);
    // MB(LungCancer) ⊇ {Smoking, TbOrCa, Tuberculosis}.
    let lc = blanket_of("LungCancer");
    for want in ["Smoking", "TbOrCa", "Tuberculosis"] {
        assert!(lc.contains(&want.to_string()), "MB(LungCancer) missing {want}: {lc:?}");
    }

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    structure::learn_gs(
        &mut net,
        &cases,
        &ids,
        &structure::GsOptions {
            hill: structure::HillClimbOptions {
                score: ScoreKind::Bdeu { ess: 10.0 },
                ..Default::default()
            },
            ..Default::default()
        },
        &SearchCtrl::default(),
    )
    .unwrap();
    let learned_skel = skeleton(&edge_indices(&net, &ids));
    let true_skel = skeleton(&edge_indices(&truth, &ids));
    // Every true edge must be recovered; the deterministic TbOrCa gate can
    // induce a couple of extra spouse edges (faithfulness violation).
    for e in &true_skel {
        assert!(learned_skel.contains(e), "GS hybrid missing true edge {e:?}");
    }
    assert!(
        hamming(&learned_skel, &true_skel) <= 2,
        "GS hybrid skeleton too far: {learned_skel:?} vs {true_skel:?}"
    );
}

#[test]
fn tan_shape_and_tree_recovery() {
    // Build a known TAN: Class → all features; feature tree F1→F2→F3.
    let mut truth = Network::new("tan");
    let two = |a: &str, b: &str| vec![bn_core::State::new(a), bn_core::State::new(b)];
    let cl = truth.add_node("Class", bn_core::NodeKind::Chance, two("pos", "neg")).unwrap();
    let f1 = truth.add_node("F1", bn_core::NodeKind::Chance, two("a", "b")).unwrap();
    let f2 = truth.add_node("F2", bn_core::NodeKind::Chance, two("a", "b")).unwrap();
    let f3 = truth.add_node("F3", bn_core::NodeKind::Chance, two("a", "b")).unwrap();
    truth.add_edge(cl, f1).unwrap();
    truth.add_edge(cl, f2).unwrap();
    truth.add_edge(cl, f3).unwrap();
    truth.add_edge(f1, f2).unwrap();
    truth.add_edge(f2, f3).unwrap();
    truth.set_table(cl, bn_core::Table { data: vec![0.4, 0.6] }).unwrap();
    truth.set_table(f1, bn_core::Table { data: vec![0.9, 0.1, 0.2, 0.8] }).unwrap();
    // F2 | Class, F1 and F3 | Class, F2: strong dependence on both parents.
    truth
        .set_table(f2, bn_core::Table { data: vec![0.95, 0.05, 0.4, 0.6, 0.5, 0.5, 0.05, 0.95] })
        .unwrap();
    truth
        .set_table(f3, bn_core::Table { data: vec![0.9, 0.1, 0.3, 0.7, 0.6, 0.4, 0.1, 0.9] })
        .unwrap();
    let ids = [cl, f1, f2, f3];
    let mut rng = StdRng::seed_from_u64(21);
    let cases = generate_cases(&truth, 20_000, 0.0, &mut rng);

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    let report = structure::learn_tan(
        &mut net,
        &cases,
        &ids,
        cl,
        &structure::TanOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    // Shape: class is parentless root; every feature has the class plus at
    // most one feature parent; the tree has n_features − 1 edges.
    assert!(net.node(cl).parents.is_empty());
    assert_eq!(report.tree_edges.len(), 2);
    for &f in &[f1, f2, f3] {
        let pa = &net.node(f).parents;
        assert!(pa.contains(&cl), "feature must have class parent");
        assert!(pa.len() <= 2, "class + at most one feature parent");
    }
    // Tree skeleton matches truth: F1−F2 and F2−F3.
    let mut tree_skel: Vec<(NodeId, NodeId)> = report
        .tree_edges
        .iter()
        .map(|&(a, b)| if a < b { (a, b) } else { (b, a) })
        .collect();
    tree_skel.sort();
    let mut want = vec![
        if f1 < f2 { (f1, f2) } else { (f2, f1) },
        if f2 < f3 { (f2, f3) } else { (f3, f2) },
    ];
    want.sort();
    assert_eq!(tree_skel, want);

    // Naive Bayes shape on the same data.
    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    structure::learn_naive_bayes(&mut net, &ids, cl).unwrap();
    for &f in &[f1, f2, f3] {
        assert_eq!(net.node(f).parents, vec![cl]);
    }
}

#[test]
fn structural_em_learns_from_incomplete_data() {
    let (truth, ids) = sprinkler();
    let mut rng = StdRng::seed_from_u64(17);
    // 30% of values knocked out, mirroring the parametric EM test.
    let cases = generate_cases(&truth, 4_000, 0.3, &mut rng);

    let mut net = truth.clone();
    blank_structure(&mut net, &ids);
    let report = structure::learn_structural_em(
        &mut net,
        &cases,
        &ids,
        &structure::SemOptions::default(),
        &SearchCtrl::default(),
    )
    .unwrap();
    // Expected score is non-decreasing across outer iterations.
    for w in report.expected_score_trace.windows(2) {
        assert!(w[1] >= w[0] - 1e-6, "expected score decreased: {:?}", report.expected_score_trace);
    }
    assert!(report.outer_iterations >= 1);
    assert!(report.log_likelihood.is_finite());
    // Structure close to truth: skeleton within 1 edge.
    let learned_skel = skeleton(&edge_indices(&net, &ids));
    let true_skel = skeleton(&edge_indices(&truth, &ids));
    assert!(
        hamming(&learned_skel, &true_skel) <= 1,
        "SEM skeleton too far: {learned_skel:?} vs {true_skel:?}"
    );
    // Parameters were fitted: beliefs are not uniform.
    let mut engine = Engine::compile(&net);
    engine.set_evidence(Evidence::new());
    let b = engine.beliefs(ids[3]).unwrap();
    assert!((b[0] - 0.5).abs() > 0.05, "WetGrass beliefs look unfitted: {b:?}");
}
