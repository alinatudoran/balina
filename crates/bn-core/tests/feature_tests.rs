//! Learning, sampling, sensitivity, decisions, and file round-trips.

mod common;

use bn_core::decision::{expected_utilities, solve_influence_diagram};
use bn_core::inference::{Engine, Evidence, Finding};
use bn_core::io::{self, Document, Format};
use bn_core::learn::{
    import_cases_with, learn_counting, learn_em, read_cases, read_cases_with, sniff_delimiter,
    write_cases, CountingOptions, EmOptions, ImportOptions,
};
use bn_core::model::{Network, NodeKind, State, Table};
use bn_core::sample::{generate_cases, lw_beliefs};
use common::{asia, assert_close, sprinkler};
use rand::rngs::StdRng;
use rand::SeedableRng;

// ---------------------------------------------------------------------------
// Sampling + learning
// ---------------------------------------------------------------------------

#[test]
fn counting_recovers_cpts_from_samples() {
    let (net, _) = sprinkler();
    let mut rng = StdRng::seed_from_u64(7);
    let cases = generate_cases(&net, 10_000, 0.0, &mut rng);

    // Learn into a copy with uniform tables.
    let mut learned = net.clone();
    for (id, node) in net.nodes() {
        let out = node.out_card();
        let len = net.table_len(id);
        learned
            .set_table(id, Table { data: vec![1.0 / out as f64; len] })
            .unwrap();
    }
    learn_counting(&mut learned, &cases, &CountingOptions::default()).unwrap();
    for (id, node) in net.nodes() {
        for (a, b) in node.table.data.iter().zip(&learned.node(id).table.data) {
            assert!((a - b).abs() < 0.02, "{}: {a} vs {b}", node.name);
        }
    }
}

#[test]
fn em_recovers_with_missing_data_and_loglik_is_monotone() {
    let (net, _) = sprinkler();
    let mut rng = StdRng::seed_from_u64(11);
    let cases = generate_cases(&net, 4_000, 0.3, &mut rng);
    let mut learned = net.clone();
    // Perturb the starting point (not uniform: EM needs symmetry breaking).
    for (id, _) in net.nodes() {
        let mut t = net.node(id).table.data.clone();
        for (i, v) in t.iter_mut().enumerate() {
            *v = (*v * 0.6 + 0.2) * if i % 2 == 0 { 1.1 } else { 0.9 };
        }
        learned.set_table(id, Table { data: t }).unwrap();
        learned.normalize_table(id);
    }
    let report = learn_em(&mut learned, &cases, &EmOptions::default()).unwrap();
    // The EM invariant: log-likelihood never decreases (allow tiny numeric slack).
    for w in report.log_likelihood_trace.windows(2) {
        assert!(w[1] >= w[0] - 1e-6, "log-lik decreased: {} -> {}", w[0], w[1]);
    }
    for (id, node) in net.nodes() {
        for (a, b) in node.table.data.iter().zip(&learned.node(id).table.data) {
            assert!((a - b).abs() < 0.08, "{}: {a} vs {b}", node.name);
        }
    }
}

#[test]
fn case_csv_roundtrip() {
    let (net, _) = sprinkler();
    let mut rng = StdRng::seed_from_u64(3);
    let cases = generate_cases(&net, 50, 0.2, &mut rng);
    let mut buf = Vec::new();
    write_cases(&net, &cases, &mut buf).unwrap();
    let back = read_cases(&net, buf.as_slice()).unwrap();
    assert_eq!(back.rows, cases.rows);
    assert_eq!(back.nodes, cases.nodes);
}

#[test]
fn case_files_infer_semicolon_and_tab_delimiters() {
    let (net, _) = sprinkler();
    let comma = "Cloudy,Sprinkler,Rain,WetGrass\nyes,on,*,no\nno,off,on,yes\n";
    let expected = read_cases(&net, comma.as_bytes()).unwrap();
    assert_eq!(expected.n_cases(), 2);

    let semi = comma.replace(',', ";");
    let got = read_cases(&net, semi.as_bytes()).unwrap();
    assert_eq!(got.rows, expected.rows);
    assert_eq!(got.nodes, expected.nodes);

    let tab = comma.replace(',', "\t");
    let inferred = read_cases(&net, tab.as_bytes()).unwrap();
    assert_eq!(inferred.rows, expected.rows);
    let forced = read_cases_with(&net, tab.as_bytes(), Some(b'\t')).unwrap();
    assert_eq!(forced.rows, expected.rows);
}

#[test]
fn case_file_with_utf8_bom_parses() {
    let (net, _) = sprinkler();
    let mut data = b"\xEF\xBB\xBF".to_vec();
    data.extend_from_slice(b"Cloudy;Rain\nyes;on\nno;off\n");
    let got = read_cases(&net, data.as_slice()).unwrap();
    assert_eq!(got.n_cases(), 2);
    // The BOM must not corrupt the first header: both columns matched.
    assert_eq!(got.nodes.len(), 2);
}

#[test]
fn import_creates_discrete_and_continuous_nodes() {
    let mut net = Network::new("import");
    let csv = "Color,Flag,Height,NumCases\n\
               red,0,1.5,1\n\
               blue,1,2.5,2\n\
               red,0,3.5,1\n\
               green,1,4.5,1\n\
               blue,0,5.5,1\n\
               red,1,*,1\n\
               red,1,6.5,1\n\
               blue,1,7.5,1\n\
               red,0,8.5,1\n";
    let (cases, reports) =
        import_cases_with(&mut net, csv.as_bytes(), None, &ImportOptions::default()).unwrap();

    // Text column: sorted distinct values become states, no numeric levels.
    let color = net.find_by_name("Color").unwrap();
    let names: Vec<&str> = net.node(color).states.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["blue", "green", "red"]);
    assert!(net.node(color).states.iter().all(|s| s.value.is_none()));

    // Integer codes stay discrete, with the code as the numeric level.
    let flag = net.find_by_name("Flag").unwrap();
    let names: Vec<&str> = net.node(flag).states.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["0", "1"]);
    assert_eq!(net.node(flag).states[1].value, Some(1.0));

    // Float column: continuous, learned weighted mean/std, 4 quantile bins.
    let height = net.find_by_name("Height").unwrap();
    assert_eq!(net.node(height).n_states(), 4);
    let stats = reports
        .iter()
        .find(|r| r.name == "Height")
        .and_then(|r| r.continuous.as_ref())
        .expect("Height reported as continuous");
    // Hand-computed over (value, weight), excluding the missing row:
    let vw = [(1.5, 1.0), (2.5, 2.0), (3.5, 1.0), (4.5, 1.0), (5.5, 1.0), (6.5, 1.0), (7.5, 1.0), (8.5, 1.0)];
    let n: f64 = vw.iter().map(|&(_, w)| w).sum();
    let mean: f64 = vw.iter().map(|&(v, w)| v * w).sum::<f64>() / n;
    let var: f64 = vw.iter().map(|&(v, w)| w * (v - mean) * (v - mean)).sum::<f64>() / n;
    assert!((stats.mean - mean).abs() < 1e-12);
    assert!((stats.std - var.sqrt()).abs() < 1e-12);
    assert!((stats.n - n).abs() < 1e-12);
    // Bin levels are the per-bin weighted means, strictly increasing.
    let levels: Vec<f64> = net.node(height).states.iter().map(|s| s.value.unwrap()).collect();
    assert_eq!(levels, vec![1.5, (2.0 * 2.5 + 3.5) / 3.0, 5.0, 7.5]);
    let ci = net.node(height).continuous.as_ref().expect("Height should have ContinuousInfo");
    assert!((ci.mean - mean).abs() < 1e-12);
    assert!(!ci.edges.is_empty());

    // Case set: weights from NumCases; missing float row maps to None.
    assert_eq!(cases.weights, vec![1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
    let h_col = cases.nodes.iter().position(|&id| id == height).unwrap();
    assert_eq!(cases.rows[5][h_col], None);
    assert_eq!(cases.rows[0][h_col], Some(0)); // 1.5 → first bin
    assert_eq!(cases.rows[8][h_col], Some(3)); // 8.5 → last bin

    // Counting on the returned cases: node mean tracks the data mean.
    learn_counting(&mut net, &cases, &CountingOptions { use_experience: false }).unwrap();
    let node = net.node(height);
    let disc_mean: f64 = node
        .table
        .data
        .iter()
        .zip(&node.states)
        .map(|(p, s)| p * s.value.unwrap())
        .sum();
    assert!((disc_mean - mean).abs() < 1e-9, "{disc_mean} vs {mean}");
}

#[test]
fn import_heavy_ties_shrink_bins_without_panic() {
    let mut net = Network::new("ties");
    let mut csv = String::from("X\n");
    for _ in 0..18 {
        csv.push_str("0\n");
    }
    csv.push_str("1.5\n2.5\n");
    let (_, reports) =
        import_cases_with(&mut net, csv.as_bytes(), None, &ImportOptions::default()).unwrap();
    // All quantile edges collapse onto 0 (= the minimum), leaving one bin.
    assert_eq!(reports[0].n_states, 1);
    assert_eq!(net.node(net.find_by_name("X").unwrap()).n_states(), 1);
}

#[test]
fn import_rejects_too_many_discrete_states_and_reuses_existing_nodes() {
    let (mut net, _) = sprinkler();
    let n_before = net.len();
    // Existing column: no node created, values map like read_cases.
    let (cases, reports) = import_cases_with(
        &mut net,
        "Cloudy\nyes\nno\n".as_bytes(),
        None,
        &ImportOptions::default(),
    )
    .unwrap();
    assert_eq!(net.len(), n_before);
    assert!(!reports[0].created);
    assert_eq!(cases.rows, vec![vec![Some(0)], vec![Some(1)]]);

    let mut csv = String::from("Tag\n");
    for i in 0..60 {
        csv.push_str(&format!("s{i}\n"));
    }
    let err = import_cases_with(&mut net, csv.as_bytes(), None, &ImportOptions::default());
    assert!(matches!(err, Err(bn_core::CaseError::TooManyStates { .. })));
}

#[test]
fn read_cases_falls_back_to_nearest_numeric_level() {
    let mut net = Network::new("bins");
    let low = State { name: "low".into(), value: Some(1.0) };
    let high = State { name: "high".into(), value: Some(5.0) };
    net.add_node("T", NodeKind::Chance, vec![low, high]).unwrap();
    let got = read_cases(&net, "T\n1.2\n4.9\nhigh\n".as_bytes()).unwrap();
    assert_eq!(got.rows, vec![vec![Some(0)], vec![Some(1)], vec![Some(1)]]);
}

#[test]
fn sniff_delimiter_picks_majority_outside_quotes() {
    assert_eq!(sniff_delimiter("\"a,b,c\";x;y"), b';');
    assert_eq!(sniff_delimiter("a\tb\tc"), b'\t');
    assert_eq!(sniff_delimiter("single_column"), b',');
    assert_eq!(sniff_delimiter("a,b;c"), b','); // tie falls back to comma
}

#[test]
fn likelihood_weighting_close_to_exact() {
    let (net, [_c, _s, r, w]) = sprinkler();
    let mut ev = Evidence::new();
    ev.set(w, Finding::Hard(0));
    let mut rng = StdRng::seed_from_u64(42);
    let approx = lw_beliefs(&net, &ev, 30_000, &mut rng).unwrap();
    let mut engine = Engine::compile(&net);
    engine.set_evidence(ev);
    let exact = engine.beliefs(r).unwrap();
    assert_close(&approx[r], &exact, 0.02, "lw rain");
}

// ---------------------------------------------------------------------------
// Sensitivity
// ---------------------------------------------------------------------------

#[test]
fn sensitivity_ranks_direct_neighbors_high() {
    let net = asia();
    let target = net.find_by_name("TbOrCa").unwrap();
    let mut engine = Engine::compile(&net);
    let candidates: Vec<_> = net
        .nodes()
        .map(|(id, _)| id)
        .filter(|&id| id != target)
        .collect();
    let rows =
        bn_core::sensitivity::sensitivity_to_findings(&mut engine, &net, target, &candidates)
            .unwrap();
    assert_eq!(rows.len(), candidates.len());
    // XRay is the strongest single indicator of TbOrCa in Asia.
    let top = &rows[0];
    assert_eq!(net.node(top.node).name, "XRay");
    assert!(top.mutual_info > 0.0);
    // MI must be symmetric-ish sane: no NaNs, sorted descending.
    for w in rows.windows(2) {
        assert!(w[0].mutual_info >= w[1].mutual_info);
    }
    // Evidence restored.
    assert!(engine.evidence().is_empty());
}

// ---------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------

/// Umbrella network: Weather → Forecast (observed) → Umbrella decision;
/// Utility(Weather, Umbrella).
fn umbrella() -> (Network, [bn_core::NodeId; 4]) {
    let mut net = Network::new("umbrella");
    let weather = net
        .add_node("Weather", NodeKind::Chance, vec![State::new("rain"), State::new("sun")])
        .unwrap();
    let forecast = net
        .add_node(
            "Forecast",
            NodeKind::Chance,
            vec![State::new("rainy"), State::new("sunny")],
        )
        .unwrap();
    let take = net
        .add_node(
            "Umbrella",
            NodeKind::Decision,
            vec![State::new("take"), State::new("leave")],
        )
        .unwrap();
    let util = net.add_node("Satisfaction", NodeKind::Utility, vec![]).unwrap();
    net.add_edge(weather, forecast).unwrap();
    net.add_edge(forecast, take).unwrap(); // informational link
    net.add_edge(weather, util).unwrap();
    net.add_edge(take, util).unwrap();
    net.set_table(weather, Table { data: vec![0.3, 0.7] }).unwrap();
    net.set_table(forecast, Table { data: vec![0.8, 0.2, 0.15, 0.85] }).unwrap();
    // Utility rows over (Weather, Umbrella): rain/take, rain/leave, sun/take, sun/leave
    net.set_table(util, Table { data: vec![70.0, 0.0, 75.0, 100.0] }).unwrap();
    (net, [weather, forecast, take, util])
}

/// Brute-force the optimal policy by enumerating all forecast→action maps.
fn umbrella_brute_meu(net: &Network) -> (f64, Vec<usize>) {
    let w = [0.3, 0.7]; // weather
    let f_given_w = [[0.8, 0.2], [0.15, 0.85]];
    let u = |weather: usize, act: usize| [[70.0, 0.0], [75.0, 100.0]][weather][act];
    let _ = net;
    let mut best = (f64::NEG_INFINITY, vec![0, 0]);
    for a_rainy in 0..2 {
        for a_sunny in 0..2 {
            let mut eu = 0.0;
            for weather in 0..2 {
                for fc in 0..2 {
                    let act = if fc == 0 { a_rainy } else { a_sunny };
                    eu += w[weather] * f_given_w[weather][fc] * u(weather, act);
                }
            }
            if eu > best.0 {
                best = (eu, vec![a_rainy, a_sunny]);
            }
        }
    }
    best
}

#[test]
fn influence_diagram_matches_brute_force() {
    let (net, [_, forecast, take, _]) = umbrella();
    let (brute_meu, brute_policy) = umbrella_brute_meu(&net);
    let sol = solve_influence_diagram(&net, &Evidence::new()).unwrap();
    assert!((sol.meu - brute_meu).abs() < 1e-9, "{} vs {brute_meu}", sol.meu);
    assert_eq!(sol.policies.len(), 1);
    let pol = &sol.policies[0];
    assert_eq!(pol.decision, take);
    assert_eq!(pol.domain, vec![forecast]);
    assert_eq!(pol.best, brute_policy);
}

#[test]
fn expected_utilities_per_choice_given_forecast() {
    let (net, [_, forecast, take, _]) = umbrella();
    let mut engine = Engine::compile(&net);
    engine.set_finding(forecast, Finding::Hard(0)).unwrap(); // forecast rainy
    let eu = expected_utilities(&mut engine, &net, take).unwrap();
    // P(rain | rainy forecast) = 0.24/0.345; EU(take) = p·70 + (1-p)·75
    let p = 0.24 / 0.345;
    let want_take = p * 70.0 + (1.0 - p) * 75.0;
    let want_leave = p * 0.0 + (1.0 - p) * 100.0;
    assert!((eu[0].unwrap() - want_take).abs() < 1e-9);
    assert!((eu[1].unwrap() - want_leave).abs() < 1e-9);
    // With a rainy forecast, taking the umbrella wins.
    assert!(eu[0].unwrap() > eu[1].unwrap());
    // Evidence restored.
    assert!(engine.evidence().get(forecast).is_some());
    assert!(engine.evidence().get(take).is_none());
}

// ---------------------------------------------------------------------------
// File formats
// ---------------------------------------------------------------------------

fn beliefs_by_name(net: &Network) -> Vec<(String, Vec<f64>)> {
    let mut engine = Engine::compile(net);
    let all = engine.all_beliefs().unwrap();
    let mut out: Vec<(String, Vec<f64>)> = all
        .iter()
        .map(|(id, b)| (net.node(id).name.clone(), b.clone()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn native_json_roundtrip() {
    let doc = Document { network: asia(), visual: Default::default() };
    let (text, warn) = io::save_str(&doc, Format::NativeJson).unwrap();
    assert!(warn.is_empty());
    let back = io::load_str(&text, Format::NativeJson).unwrap();
    let a = beliefs_by_name(&doc.network);
    let b = beliefs_by_name(&back.network);
    for ((na, ba), (nb, bb)) in a.iter().zip(&b) {
        assert_eq!(na, nb);
        assert_close(ba, bb, 1e-12, na);
    }
    // Stable: save(load(x)) == save(x)
    let (text2, _) = io::save_str(&back, Format::NativeJson).unwrap();
    assert_eq!(text, text2);
}

#[test]
fn xmlbif_roundtrip() {
    let doc = Document { network: asia(), visual: Default::default() };
    let (text, _) = io::save_str(&doc, Format::Xmlbif).unwrap();
    let back = io::load_str(&text, Format::Xmlbif).unwrap();
    let a = beliefs_by_name(&doc.network);
    let b = beliefs_by_name(&back.network);
    assert_eq!(a.len(), b.len());
    for ((na, ba), (nb, bb)) in a.iter().zip(&b) {
        assert_eq!(na, nb);
        assert_close(ba, bb, 1e-12, na);
    }
}

#[test]
fn xdsl_roundtrip_preserves_influence_diagram() {
    let (net, _) = umbrella();
    let doc = Document { network: net, visual: Default::default() };
    let (text, _) = io::save_str(&doc, Format::Xdsl).unwrap();
    let back = io::load_str(&text, Format::Xdsl).unwrap();
    let sol_a = solve_influence_diagram(&doc.network, &Evidence::new()).unwrap();
    let sol_b = solve_influence_diagram(&back.network, &Evidence::new()).unwrap();
    assert!((sol_a.meu - sol_b.meu).abs() < 1e-9);
    assert_eq!(sol_a.policies[0].best, sol_b.policies[0].best);
}

#[test]
fn continuous_info_survives_native_roundtrip() {
    let mut net = Network::new("ct");
    let csv = "Height,NumCases\n\
               1.5,1\n2.5,2\n3.5,1\n4.5,1\n5.5,1\n6.5,1\n7.5,1\n8.5,1\n";
    import_cases_with(&mut net, csv.as_bytes(), None, &ImportOptions::default()).unwrap();
    let height = net.find_by_name("Height").unwrap();
    let ci_before = net.node(height).continuous.clone().expect("should have ContinuousInfo");

    let doc = Document { network: net, visual: Default::default() };
    let (text, _) = io::save_str(&doc, Format::NativeJson).unwrap();
    let back = io::load_str(&text, Format::NativeJson).unwrap();
    let h2 = back.network.find_by_name("Height").unwrap();
    let ci_after = back.network.node(h2).continuous.as_ref().expect("ContinuousInfo must survive round-trip");

    assert!((ci_before.mean - ci_after.mean).abs() < 1e-12);
    assert!((ci_before.std - ci_after.std).abs() < 1e-12);
    assert_eq!(ci_before.edges, ci_after.edges);
}
