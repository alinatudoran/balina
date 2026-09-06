//! Expectation-maximization for incomplete case data, driven by the
//! junction-tree engine's family posteriors.

use std::collections::HashMap;

use crate::error::{InferenceError, LearnError};
use crate::factor::encode_index;
use crate::inference::{Engine, Evidence, Finding};
use crate::learn::cases::CaseSet;
use crate::model::{Network, NodeId, NodeKind, Table};

#[derive(Clone, Debug)]
pub struct EmOptions {
    pub max_iters: usize,
    /// Stop when the log-likelihood improves by less than this.
    pub tol: f64,
    /// Dirichlet smoothing added to every expected count cell.
    pub pseudo_count: f64,
}

impl Default for EmOptions {
    fn default() -> Self {
        EmOptions { max_iters: 50, tol: 1e-4, pseudo_count: 0.01 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EmReport {
    pub iterations: usize,
    pub log_likelihood_trace: Vec<f64>,
    /// Cases with zero probability under the model, skipped.
    pub conflicting_cases: usize,
}

pub fn learn_em(
    net: &mut Network,
    cases: &CaseSet,
    opts: &EmOptions,
) -> Result<EmReport, LearnError> {
    let chance: Vec<NodeId> = net
        .nodes()
        .filter(|(_, n)| n.kind == NodeKind::Chance)
        .map(|(id, _)| id)
        .collect();
    if chance.is_empty() {
        return Err(LearnError::NothingToLearn);
    }
    // Deduplicate identical evidence patterns (big win on real data).
    let mut unique: HashMap<Vec<Option<usize>>, f64> = HashMap::new();
    for (row, &w) in cases.rows.iter().zip(&cases.weights) {
        *unique.entry(row.clone()).or_insert(0.0) += w;
    }
    let unique: Vec<(Vec<Option<usize>>, f64)> = unique.into_iter().collect();

    let mut report = EmReport::default();
    let mut prev_ll = f64::NEG_INFINITY;
    for iter in 0..opts.max_iters {
        let mut engine = Engine::compile(net);
        // Expected counts per chance node, in table layout.
        let mut counts: HashMap<NodeId, Vec<f64>> = chance
            .iter()
            .map(|&id| (id, vec![opts.pseudo_count; net.table_len(id)]))
            .collect();
        let mut ll = 0.0;
        let mut conflicts = 0usize;
        for (row, w) in &unique {
            let mut ev = Evidence::new();
            for (col, v) in row.iter().enumerate() {
                if let Some(s) = v {
                    ev.set(cases.nodes[col], Finding::Hard(*s));
                }
            }
            engine.set_evidence(ev);
            match engine.log_prob_of_findings() {
                Ok(lp) => ll += w * lp,
                Err(InferenceError::ConflictingEvidence) => {
                    conflicts += 1;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            for &id in &chance {
                let post = engine.family_posterior(id).map_err(LearnError::Inference)?;
                fold_family(net, &engine, id, &post, *w, counts.get_mut(&id).unwrap());
            }
        }
        report.conflicting_cases = conflicts;
        report.log_likelihood_trace.push(ll);
        report.iterations = iter + 1;
        // M-step.
        for &id in &chance {
            let out = net.node(id).out_card();
            let mut data = counts.remove(&id).unwrap();
            for row in data.chunks_mut(out) {
                let s: f64 = row.iter().sum();
                if s > 0.0 {
                    for v in row.iter_mut() {
                        *v /= s;
                    }
                } else {
                    for v in row.iter_mut() {
                        *v = 1.0 / out as f64;
                    }
                }
            }
            net.set_table(id, Table { data }).unwrap();
        }
        if (ll - prev_ll).abs() < opts.tol {
            break;
        }
        prev_ll = ll;
    }
    Ok(report)
}

/// Accumulate `w · P(family | e)` into a node's table-layout count array.
fn fold_family(
    net: &Network,
    engine: &Engine,
    id: NodeId,
    post: &crate::factor::Factor,
    w: f64,
    counts: &mut [f64],
) {
    // Table axes: [parents in node order..., self].
    let cn = engine.compiled();
    let mut axis_vars: Vec<crate::factor::VarId> =
        net.node(id).parents.iter().map(|&p| cn.var_of[p]).collect();
    axis_vars.push(cn.var_of[id]);
    let axis_cards: Vec<usize> = net
        .node(id)
        .parents
        .iter()
        .map(|&p| net.node(p).n_states())
        .chain([net.node(id).n_states()])
        .collect();
    // Position of each table axis in the (sorted) posterior factor.
    let pos: Vec<usize> = axis_vars
        .iter()
        .map(|v| post.vars.iter().position(|w| w == v).unwrap())
        .collect();
    for (i, &p) in post.data.iter().enumerate() {
        if p == 0.0 {
            continue;
        }
        let a = crate::factor::decode_index(i, &post.cards);
        let t: Vec<usize> = pos.iter().map(|&j| a[j]).collect();
        counts[encode_index(&t, &axis_cards)] += w * p;
    }
}
