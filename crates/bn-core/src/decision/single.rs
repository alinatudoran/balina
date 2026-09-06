//! Expected utility per choice of one decision node, given the evidence
//! currently entered — displayed as the decision node's belief bars.
//! For multiple decisions this is a myopic view (other decisions are
//! marginalized uniformly); use `ve_id` for exact policies.

use crate::error::{IdError, InferenceError};
use crate::factor::decode_index;
use crate::inference::{Engine, Finding};
use crate::model::{Network, NodeId, NodeKind};

/// EU of each state of `decision` under the engine's current evidence.
/// `None` for choices that conflict with the evidence. The engine's evidence
/// is restored before returning.
pub fn expected_utilities(
    engine: &mut Engine,
    net: &Network,
    decision: NodeId,
) -> Result<Vec<Option<f64>>, IdError> {
    let utilities: Vec<NodeId> = net
        .nodes()
        .filter(|(_, n)| n.kind == NodeKind::Utility)
        .map(|(id, _)| id)
        .collect();
    if utilities.is_empty() {
        return Err(IdError::NoUtilities);
    }
    let saved = engine.evidence().clone();
    let k = net.node(decision).n_states();
    let mut out = Vec::with_capacity(k);
    for d in 0..k {
        engine.set_evidence(saved.clone());
        engine.set_finding(decision, Finding::Hard(d)).map_err(IdError::Inference)?;
        let mut eu = 0.0;
        let mut ok = true;
        for &u in &utilities {
            match utility_expectation(engine, net, u) {
                Ok(v) => eu += v,
                Err(InferenceError::ConflictingEvidence) => {
                    ok = false;
                    break;
                }
                Err(e) => {
                    engine.set_evidence(saved.clone());
                    return Err(IdError::Inference(e));
                }
            }
        }
        out.push(if ok { Some(eu) } else { None });
    }
    engine.set_evidence(saved);
    Ok(out)
}

/// E[U | evidence] for one utility node: Σ_pa P(pa | e) · U(pa).
pub fn utility_expectation(
    engine: &mut Engine,
    net: &Network,
    utility: NodeId,
) -> Result<f64, InferenceError> {
    let node = net.node(utility);
    if node.parents.is_empty() {
        return Ok(node.table.data[0]);
    }
    let joint = engine.joint_posterior(&node.parents)?;
    // Map each joint assignment (over sorted vars) to the utility table index
    // (parents in declared order).
    let cn = engine.compiled();
    let pcards = net.parent_cards(utility);
    let pos: Vec<usize> = node
        .parents
        .iter()
        .map(|&p| {
            let v = cn.var_of[p];
            joint.vars.iter().position(|w| *w == v).unwrap()
        })
        .collect();
    let mut eu = 0.0;
    for (i, &pr) in joint.data.iter().enumerate() {
        if pr == 0.0 {
            continue;
        }
        let a = decode_index(i, &joint.cards);
        let t: Vec<usize> = pos.iter().map(|&j| a[j]).collect();
        eu += pr * node.table.data[crate::factor::encode_index(&t, &pcards)];
    }
    Ok(eu)
}
