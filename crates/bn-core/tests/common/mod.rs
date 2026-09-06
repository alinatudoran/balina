//! Shared test networks.

use bn_core::model::{Network, NodeId, NodeKind, State, Table};

pub fn two(name: &str) -> Vec<State> {
    let _ = name;
    vec![State::new("yes"), State::new("no")]
}

/// The classic Sprinkler network (Cloudy → Sprinkler, Rain → WetGrass).
///
/// Hand-checkable arithmetic:
///   P(S=on) = 0.5·0.1 + 0.5·0.5 = 0.30
///   P(R=on) = 0.5·0.8 + 0.5·0.2 = 0.50
pub fn sprinkler() -> (Network, [NodeId; 4]) {
    let mut net = Network::new("sprinkler");
    let on_off = || vec![State::new("on"), State::new("off")];
    let c = net.add_node("Cloudy", NodeKind::Chance, two("c")).unwrap();
    let s = net.add_node("Sprinkler", NodeKind::Chance, on_off()).unwrap();
    let r = net.add_node("Rain", NodeKind::Chance, on_off()).unwrap();
    let w = net.add_node("WetGrass", NodeKind::Chance, two("w")).unwrap();
    net.add_edge(c, s).unwrap();
    net.add_edge(c, r).unwrap();
    net.add_edge(s, w).unwrap();
    net.add_edge(r, w).unwrap();
    net.set_table(c, Table { data: vec![0.5, 0.5] }).unwrap();
    // Sprinkler | Cloudy: cloudy → 0.1 on; clear → 0.5 on
    net.set_table(s, Table { data: vec![0.1, 0.9, 0.5, 0.5] }).unwrap();
    // Rain | Cloudy: cloudy → 0.8; clear → 0.2
    net.set_table(r, Table { data: vec![0.8, 0.2, 0.2, 0.8] }).unwrap();
    // WetGrass | Sprinkler, Rain (rows: on,on / on,off / off,on / off,off)
    net.set_table(
        w,
        Table { data: vec![0.99, 0.01, 0.9, 0.1, 0.9, 0.1, 0.0, 1.0] },
    )
    .unwrap();
    (net, [c, s, r, w])
}

/// Asia / Chest Clinic (Lauritzen & Spiegelhalter 1988), 8 nodes.
pub fn asia() -> Network {
    let mut net = Network::new("asia");
    let n = |net: &mut Network, name: &str| {
        net.add_node(name, NodeKind::Chance, two(name)).unwrap()
    };
    let asia = n(&mut net, "VisitAsia");
    let tub = n(&mut net, "Tuberculosis");
    let smoke = n(&mut net, "Smoking");
    let lung = n(&mut net, "LungCancer");
    let bronc = n(&mut net, "Bronchitis");
    let either = n(&mut net, "TbOrCa");
    let xray = n(&mut net, "XRay");
    let dysp = n(&mut net, "Dyspnea");
    net.add_edge(asia, tub).unwrap();
    net.add_edge(smoke, lung).unwrap();
    net.add_edge(smoke, bronc).unwrap();
    net.add_edge(tub, either).unwrap();
    net.add_edge(lung, either).unwrap();
    net.add_edge(either, xray).unwrap();
    net.add_edge(either, dysp).unwrap();
    net.add_edge(bronc, dysp).unwrap();
    net.set_table(asia, Table { data: vec![0.01, 0.99] }).unwrap();
    net.set_table(tub, Table { data: vec![0.05, 0.95, 0.01, 0.99] }).unwrap();
    net.set_table(smoke, Table { data: vec![0.5, 0.5] }).unwrap();
    net.set_table(lung, Table { data: vec![0.1, 0.9, 0.01, 0.99] }).unwrap();
    net.set_table(bronc, Table { data: vec![0.6, 0.4, 0.3, 0.7] }).unwrap();
    // either = tub OR lung (rows: yy, yn, ny, nn)
    net.set_table(
        either,
        Table { data: vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0] },
    )
    .unwrap();
    net.set_table(xray, Table { data: vec![0.98, 0.02, 0.05, 0.95] }).unwrap();
    // dysp | either, bronc (rows: yy, yn, ny, nn)
    net.set_table(
        dysp,
        Table { data: vec![0.9, 0.1, 0.7, 0.3, 0.8, 0.2, 0.1, 0.9] },
    )
    .unwrap();
    net
}

// Not every test binary that includes `common` uses this.
#[allow(dead_code)]
pub fn assert_close(a: &[f64], b: &[f64], tol: f64, ctx: &str) {
    assert_eq!(a.len(), b.len(), "{ctx}: length mismatch");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!(
            (x - y).abs() < tol,
            "{ctx}: state {i}: {x} vs {y} (tol {tol})"
        );
    }
}
