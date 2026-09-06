//! Likelihood weighting: approximate posterior marginals under evidence.
//! Also the scalability escape hatch when a network's treewidth defeats the
//! junction tree.

use rand::Rng;
use slotmap::SecondaryMap;

use crate::error::InferenceError;
use crate::factor::encode_index;
use crate::inference::{Evidence, Finding};
use crate::model::{Network, NodeId, NodeKind};

pub struct WeightedSample {
    pub states: SecondaryMap<NodeId, usize>,
    pub weight: f64,
}

/// Approximate P(node | evidence) for every chance/decision node from `n`
/// weighted samples.
pub fn lw_beliefs(
    net: &Network,
    evidence: &Evidence,
    n: usize,
    rng: &mut impl Rng,
) -> Result<SecondaryMap<NodeId, Vec<f64>>, InferenceError> {
    let order: Vec<NodeId> = net
        .topo_order()
        .into_iter()
        .filter(|&id| net.node(id).kind != NodeKind::Utility)
        .collect();
    let mut acc: SecondaryMap<NodeId, Vec<f64>> = SecondaryMap::new();
    for &id in &order {
        acc.insert(id, vec![0.0; net.node(id).n_states()]);
    }
    let mut total = 0.0;
    for _ in 0..n {
        let mut states: SecondaryMap<NodeId, usize> = SecondaryMap::new();
        let mut w = 1.0;
        for &id in &order {
            let node = net.node(id);
            let probs: Vec<f64> = match node.kind {
                NodeKind::Decision => {
                    vec![1.0 / node.n_states() as f64; node.n_states()]
                }
                _ => {
                    let cards = net.parent_cards(id);
                    let pstates: Vec<usize> =
                        node.parents.iter().map(|&p| states[p]).collect();
                    let row = encode_index(&pstates, &cards);
                    net.table_row(id, row).to_vec()
                }
            };
            match evidence.get(id) {
                Some(Finding::Hard(s)) => {
                    w *= probs[*s];
                    states.insert(id, *s);
                }
                Some(Finding::Likelihood(l)) => {
                    let s = super::forward::sample_categorical(&probs, rng);
                    w *= l.get(s).copied().unwrap_or(0.0);
                    states.insert(id, s);
                }
                None => {
                    let s = super::forward::sample_categorical(&probs, rng);
                    states.insert(id, s);
                }
            }
            if w == 0.0 {
                break;
            }
        }
        if w > 0.0 {
            total += w;
            for &id in &order {
                acc[id][states[id]] += w;
            }
        }
    }
    if total <= 0.0 {
        return Err(InferenceError::ConflictingEvidence);
    }
    for (_, v) in acc.iter_mut() {
        for x in v.iter_mut() {
            *x /= total;
        }
    }
    Ok(acc)
}
