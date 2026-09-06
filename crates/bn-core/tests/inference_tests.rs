mod common;

use bn_core::inference::{posterior_enumeration, Engine, Evidence, Finding};
use bn_core::model::{Network, NodeKind, State, Table};
use common::{asia, assert_close, sprinkler};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

#[test]
fn sprinkler_priors_hand_checked() {
    let (net, [c, s, r, _w]) = sprinkler();
    let mut engine = Engine::compile(&net);
    assert_close(&engine.beliefs(c).unwrap(), &[0.5, 0.5], 1e-9, "cloudy");
    // P(S=on) = 0.5·0.1 + 0.5·0.5 = 0.30
    assert_close(&engine.beliefs(s).unwrap(), &[0.30, 0.70], 1e-9, "sprinkler");
    // P(R=on) = 0.5·0.8 + 0.5·0.2 = 0.50
    assert_close(&engine.beliefs(r).unwrap(), &[0.50, 0.50], 1e-9, "rain");
}

#[test]
fn sprinkler_posterior_vs_enumeration() {
    let (net, [c, s, r, w]) = sprinkler();
    let mut engine = Engine::compile(&net);
    engine.set_finding(w, Finding::Hard(0)).unwrap();
    let mut ev = Evidence::new();
    ev.set(w, Finding::Hard(0));
    for node in [c, s, r] {
        let jt = engine.beliefs(node).unwrap();
        let brute = posterior_enumeration(&net, &ev, node).unwrap();
        assert_close(&jt, &brute, 1e-9, "wet grass evidence");
    }
    // "Explaining away": also condition on rain, sprinkler belief must drop.
    let p_s_given_w = engine.beliefs(s).unwrap()[0];
    engine.set_finding(r, Finding::Hard(0)).unwrap();
    let p_s_given_wr = engine.beliefs(s).unwrap()[0];
    assert!(p_s_given_wr < p_s_given_w, "{p_s_given_wr} !< {p_s_given_w}");
}

#[test]
fn asia_priors_match_published_values() {
    let net = asia();
    let mut engine = Engine::compile(&net);
    let b = |e: &mut Engine, name: &str| {
        e.beliefs(net.find_by_name(name).unwrap()).unwrap()[0]
    };
    // Lauritzen & Spiegelhalter's chest-clinic marginals.
    assert!((b(&mut engine, "Tuberculosis") - 0.0104).abs() < 1e-4);
    assert!((b(&mut engine, "LungCancer") - 0.055).abs() < 1e-3);
    assert!((b(&mut engine, "Bronchitis") - 0.45).abs() < 1e-3);
    assert!((b(&mut engine, "TbOrCa") - 0.064828).abs() < 1e-5);
    assert!((b(&mut engine, "XRay") - 0.11029).abs() < 1e-4);
    assert!((b(&mut engine, "Dyspnea") - 0.4360).abs() < 1e-3);
}

#[test]
fn asia_with_evidence_vs_enumeration() {
    let net = asia();
    let xray = net.find_by_name("XRay").unwrap();
    let dysp = net.find_by_name("Dyspnea").unwrap();
    let mut ev = Evidence::new();
    ev.set(xray, Finding::Hard(0));
    ev.set(dysp, Finding::Hard(0));
    let mut engine = Engine::compile(&net);
    engine.set_evidence(ev.clone());
    for (id, _) in net.nodes() {
        let jt = engine.beliefs(id).unwrap();
        let brute = posterior_enumeration(&net, &ev, id).unwrap();
        assert_close(&jt, &brute, 1e-9, &net.node(id).name);
    }
}

#[test]
fn likelihood_evidence_vs_enumeration() {
    let net = asia();
    let xray = net.find_by_name("XRay").unwrap();
    let smoke = net.find_by_name("Smoking").unwrap();
    let mut ev = Evidence::new();
    ev.set(xray, Finding::Likelihood(vec![0.8, 0.3]));
    ev.set(smoke, Finding::Likelihood(vec![2.0, 1.0])); // unnormalized is fine
    let mut engine = Engine::compile(&net);
    engine.set_evidence(ev.clone());
    for (id, _) in net.nodes() {
        let jt = engine.beliefs(id).unwrap();
        let brute = posterior_enumeration(&net, &ev, id).unwrap();
        assert_close(&jt, &brute, 1e-9, &net.node(id).name);
    }
}

#[test]
fn prob_of_findings_matches_hand_value() {
    let (net, [_c, _s, _r, w]) = sprinkler();
    let mut engine = Engine::compile(&net);
    engine.set_finding(w, Finding::Hard(0)).unwrap();
    // P(W=on) = Σ_{c,s,r} P(c)P(s|c)P(r|c)P(W=on|s,r)
    // cloudy:  0.5·(0.1·0.8·0.99 + 0.1·0.2·0.9 + 0.9·0.8·0.9 + 0) = 0.37260
    // clear:   0.5·(0.5·0.2·0.99 + 0.5·0.8·0.9 + 0.5·0.2·0.9 + 0) = 0.27450
    // wait — recompute in-test instead:
    let mut p = 0.0;
    for c in 0..2 {
        for s in 0..2 {
            for r in 0..2 {
                let pc = [0.5, 0.5][c];
                let ps = [[0.1, 0.9], [0.5, 0.5]][c][s];
                let pr = [[0.8, 0.2], [0.2, 0.8]][c][r];
                let pw = [[0.99, 0.9], [0.9, 0.0]][s][r];
                p += pc * ps * pr * pw;
            }
        }
    }
    let lp = engine.log_prob_of_findings().unwrap();
    assert!((lp.exp() - p).abs() < 1e-12, "{} vs {p}", lp.exp());
}

#[test]
fn conflicting_evidence_is_an_error() {
    let mut net = Network::new("det");
    let a = net
        .add_node("A", NodeKind::Chance, vec![State::new("y"), State::new("n")])
        .unwrap();
    let b = net
        .add_node("B", NodeKind::Chance, vec![State::new("y"), State::new("n")])
        .unwrap();
    net.add_edge(a, b).unwrap();
    // B is a copy of A.
    net.set_table(b, Table { data: vec![1.0, 0.0, 0.0, 1.0] }).unwrap();
    let mut engine = Engine::compile(&net);
    engine.set_finding(a, Finding::Hard(0)).unwrap();
    engine.set_finding(b, Finding::Hard(1)).unwrap();
    assert!(matches!(
        engine.beliefs(a),
        Err(bn_core::InferenceError::ConflictingEvidence)
    ));
    // Retracting the conflict recovers.
    engine.retract_finding(b);
    assert_close(&engine.beliefs(b).unwrap(), &[1.0, 0.0], 1e-12, "recovered");
}

// ---------------------------------------------------------------------------
// Randomized differential testing: junction tree vs brute-force enumeration.
// ---------------------------------------------------------------------------

fn random_net(rng: &mut StdRng) -> Network {
    let n = rng.random_range(3..8);
    let mut net = Network::new("rand");
    let ids: Vec<_> = (0..n)
        .map(|i| {
            let k = rng.random_range(2..4);
            let states = (0..k).map(|s| State::new(format!("s{s}"))).collect();
            net.add_node(&format!("N{i}"), NodeKind::Chance, states).unwrap()
        })
        .collect();
    for i in 0..n {
        for j in i + 1..n {
            if rng.random::<f64>() < 0.4 {
                net.add_edge(ids[i], ids[j]).unwrap();
            }
        }
    }
    for &id in &ids {
        let len = net.table_len(id);
        let out = net.node(id).out_card();
        let mut data: Vec<f64> = (0..len).map(|_| rng.random::<f64>() + 0.01).collect();
        for row in data.chunks_mut(out) {
            let s: f64 = row.iter().sum();
            row.iter_mut().for_each(|v| *v /= s);
        }
        net.set_table(id, Table { data }).unwrap();
    }
    net
}

#[test]
fn junction_tree_equals_enumeration_on_random_nets() {
    for seed in 0..40u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let net = random_net(&mut rng);
        let mut ev = Evidence::new();
        for (id, node) in net.nodes() {
            let roll: f64 = rng.random();
            if roll < 0.15 {
                ev.set(id, Finding::Hard(rng.random_range(0..node.n_states())));
            } else if roll < 0.3 {
                let l: Vec<f64> =
                    (0..node.n_states()).map(|_| rng.random::<f64>()).collect();
                ev.set(id, Finding::Likelihood(l));
            }
        }
        let mut engine = Engine::compile(&net);
        engine.set_evidence(ev.clone());
        for (id, _) in net.nodes() {
            match (engine.beliefs(id), posterior_enumeration(&net, &ev, id)) {
                (Ok(jt), Ok(brute)) => {
                    assert_close(&jt, &brute, 1e-9, &format!("seed {seed}"))
                }
                (Err(_), Err(_)) => {} // both agree the evidence conflicts
                (a, b) => panic!("seed {seed}: disagreement: {a:?} vs {b:?}"),
            }
        }
    }
}

#[test]
fn family_posterior_consistent_with_beliefs() {
    let net = asia();
    let dysp = net.find_by_name("Dyspnea").unwrap();
    let xray = net.find_by_name("XRay").unwrap();
    let mut engine = Engine::compile(&net);
    engine.set_finding(xray, Finding::Hard(0)).unwrap();
    let fam = engine.family_posterior(dysp).unwrap();
    let var = engine.var_of(dysp).unwrap();
    let marg = fam.marginalize_to(&[var]);
    assert_close(&marg.data, &engine.beliefs(dysp).unwrap(), 1e-9, "family vs belief");
}

#[test]
fn joint_posterior_of_non_clique_pair() {
    let net = asia();
    let asia_n = net.find_by_name("VisitAsia").unwrap();
    let dysp = net.find_by_name("Dyspnea").unwrap();
    let mut engine = Engine::compile(&net);
    let joint = engine.joint_posterior(&[asia_n, dysp]).unwrap();
    assert!((joint.sum() - 1.0).abs() < 1e-9);
    // Marginals of the joint must match single-node beliefs.
    let va = engine.var_of(asia_n).unwrap();
    let ma = joint.marginalize_to(&[va]);
    assert_close(&ma.data, &engine.beliefs(asia_n).unwrap(), 1e-9, "joint marginal");
}
