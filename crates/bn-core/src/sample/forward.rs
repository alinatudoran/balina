//! Ancestral sampling and case generation.

use rand::{Rng, RngExt};
use slotmap::SecondaryMap;

use crate::factor::encode_index;
use crate::learn::cases::CaseSet;
use crate::model::{Network, NodeId, NodeKind};

pub(crate) fn sample_categorical(probs: &[f64], rng: &mut impl Rng) -> usize {
    let total: f64 = probs.iter().sum();
    if total <= 0.0 {
        return 0;
    }
    let mut u: f64 = rng.random::<f64>() * total;
    for (i, &p) in probs.iter().enumerate() {
        u -= p;
        if u <= 0.0 {
            return i;
        }
    }
    probs.len() - 1
}

/// Sample every chance and decision node once, in topological order.
/// Decision nodes are sampled uniformly.
pub fn forward_sample(net: &Network, rng: &mut impl Rng) -> SecondaryMap<NodeId, usize> {
    let mut sample: SecondaryMap<NodeId, usize> = SecondaryMap::new();
    for id in net.topo_order() {
        let node = net.node(id);
        match node.kind {
            NodeKind::Utility => {}
            NodeKind::Decision => {
                sample.insert(id, rng.random_range(0..node.n_states()));
            }
            NodeKind::Chance => {
                let cards = net.parent_cards(id);
                let pstates: Vec<usize> =
                    node.parents.iter().map(|&p| sample[p]).collect();
                let row = encode_index(&pstates, &cards);
                let s = sample_categorical(net.table_row(id, row), rng);
                sample.insert(id, s);
            }
        }
    }
    sample
}

/// Generate `n` cases over all chance/decision nodes; each value is knocked
/// out (made missing) independently with probability `missing_rate`.
pub fn generate_cases(
    net: &Network,
    n: usize,
    missing_rate: f64,
    rng: &mut impl Rng,
) -> CaseSet {
    let nodes: Vec<NodeId> = net
        .topo_order()
        .into_iter()
        .filter(|&id| net.node(id).kind != NodeKind::Utility)
        .collect();
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let s = forward_sample(net, rng);
        rows.push(
            nodes
                .iter()
                .map(|&id| {
                    if missing_rate > 0.0 && rng.random::<f64>() < missing_rate {
                        None
                    } else {
                        Some(s[id])
                    }
                })
                .collect(),
        );
    }
    CaseSet { nodes, weights: vec![1.0; rows.len()], rows }
}
