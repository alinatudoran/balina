//! Writes the bundled example networks into `examples/` at the repo root.
//! Run with: cargo run -p bn-core --example make_examples

use bn_core::io::{self, Document, NodeVisual, VisualInfo};
use bn_core::model::{Network, NodeKind, State, Table};

fn two() -> Vec<State> {
    vec![State::new("yes"), State::new("no")]
}

fn place(visual: &mut VisualInfo, name: &str, x: f32, y: f32) {
    visual.nodes.insert(
        name.into(),
        NodeVisual { x, y, display: Default::default(), color: None },
    );
}

fn asia() -> Document {
    let mut net = Network::new("Chest Clinic (Asia)");
    let mut n = |net: &mut Network, name: &str| net.add_node(name, NodeKind::Chance, two()).unwrap();
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
    net.set_table(either, Table { data: vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0] }).unwrap();
    net.set_table(xray, Table { data: vec![0.98, 0.02, 0.05, 0.95] }).unwrap();
    net.set_table(dysp, Table { data: vec![0.9, 0.1, 0.7, 0.3, 0.8, 0.2, 0.1, 0.9] }).unwrap();
    let mut visual = VisualInfo::default();
    place(&mut visual, "VisitAsia", 40.0, 40.0);
    place(&mut visual, "Tuberculosis", 40.0, 200.0);
    place(&mut visual, "Smoking", 480.0, 40.0);
    place(&mut visual, "LungCancer", 300.0, 200.0);
    place(&mut visual, "Bronchitis", 620.0, 200.0);
    place(&mut visual, "TbOrCa", 170.0, 360.0);
    place(&mut visual, "XRay", 40.0, 520.0);
    place(&mut visual, "Dyspnea", 400.0, 520.0);
    Document { network: net, visual }
}

fn umbrella() -> Document {
    let mut net = Network::new("Umbrella");
    let weather = net
        .add_node("Weather", NodeKind::Chance, vec![State::new("rain"), State::new("sun")])
        .unwrap();
    let forecast = net
        .add_node("Forecast", NodeKind::Chance, vec![State::new("rainy"), State::new("sunny")])
        .unwrap();
    let take = net
        .add_node("Umbrella", NodeKind::Decision, vec![State::new("take"), State::new("leave")])
        .unwrap();
    let util = net.add_node("Satisfaction", NodeKind::Utility, vec![]).unwrap();
    net.add_edge(weather, forecast).unwrap();
    net.add_edge(forecast, take).unwrap();
    net.add_edge(weather, util).unwrap();
    net.add_edge(take, util).unwrap();
    net.set_table(weather, Table { data: vec![0.3, 0.7] }).unwrap();
    net.set_table(forecast, Table { data: vec![0.8, 0.2, 0.15, 0.85] }).unwrap();
    net.set_table(util, Table { data: vec![70.0, 0.0, 75.0, 100.0] }).unwrap();
    let mut visual = VisualInfo::default();
    place(&mut visual, "Weather", 60.0, 60.0);
    place(&mut visual, "Forecast", 60.0, 260.0);
    place(&mut visual, "Umbrella", 360.0, 260.0);
    place(&mut visual, "Satisfaction", 360.0, 60.0);
    Document { network: net, visual }
}

fn main() {
    std::fs::create_dir_all("examples").unwrap();
    io::save(&asia(), std::path::Path::new("examples/asia.balina")).unwrap();
    io::save(&umbrella(), std::path::Path::new("examples/umbrella.balina")).unwrap();
    // Also export interchange-format variants.
    io::save(&asia(), std::path::Path::new("examples/asia.xmlbif")).unwrap();
    io::save(&umbrella(), std::path::Path::new("examples/umbrella.xdsl")).unwrap();
    println!("wrote examples/");
}
