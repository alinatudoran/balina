//! Brute-force inference by full joint enumeration. Exponential — kept
//! forever as the differential-testing oracle for the junction tree engine,
//! and usable directly on tiny networks.

use crate::error::InferenceError;
use crate::factor::decode_index;
use crate::inference::{CompiledNet, Evidence, Finding};
use crate::model::{Network, NodeId};

/// Posterior P(target | evidence) by summing the full joint.
pub fn posterior_enumeration(
    net: &Network,
    evidence: &Evidence,
    target: NodeId,
) -> Result<Vec<f64>, InferenceError> {
    let cn = CompiledNet::compile(net);
    let t = cn.var_of.get(target).ok_or(InferenceError::NotAVariable)?.0 as usize;
    let mut marginal = vec![0.0; cn.cards[t]];
    let total: usize = cn.cards.iter().product();
    // Likelihood vector per variable from evidence.
    let liks: Vec<Option<Vec<f64>>> = cn
        .order
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            evidence.get(id).map(|f| match f {
                Finding::Hard(s) => {
                    let mut v = vec![0.0; cn.cards[i]];
                    v[*s] = 1.0;
                    v
                }
                Finding::Likelihood(l) => l.clone(),
            })
        })
        .collect();
    for lin in 0..total {
        let a = decode_index(lin, &cn.cards);
        let mut w = 1.0;
        for (i, fam) in cn.families.iter().enumerate() {
            let fa: Vec<usize> = fam.vars.iter().map(|v| a[v.0 as usize]).collect();
            w *= fam.at(&fa);
            if let Some(l) = &liks[i] {
                w *= l[a[i]];
            }
            if w == 0.0 {
                break;
            }
        }
        marginal[a[t]] += w;
    }
    let s: f64 = marginal.iter().sum();
    if !(s > 0.0) {
        return Err(InferenceError::ConflictingEvidence);
    }
    Ok(marginal.into_iter().map(|v| v / s).collect())
}
