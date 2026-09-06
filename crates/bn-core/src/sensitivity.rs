//! Sensitivity to Findings: for a target node, how much would
//! a finding at each candidate node reduce the target's uncertainty?

use crate::error::InferenceError;
use crate::inference::{Engine, Finding};
use crate::model::{Network, NodeId};

#[derive(Clone, Debug)]
pub struct SensRow {
    pub node: NodeId,
    /// Mutual information I(target; node | evidence), in bits.
    pub mutual_info: f64,
    /// 100 · I / H(target | evidence).
    pub entropy_reduction_pct: f64,
    /// Expected reduction of Var(target) — only when the target's states all
    /// have numeric values.
    pub variance_reduction: Option<f64>,
}

fn entropy_bits(p: &[f64]) -> f64 {
    p.iter().filter(|&&x| x > 0.0).map(|&x| -x * x.log2()).sum()
}

fn mean_var(p: &[f64], values: &[f64]) -> (f64, f64) {
    let mean: f64 = p.iter().zip(values).map(|(&pi, &v)| pi * v).sum();
    let var: f64 = p.iter().zip(values).map(|(&pi, &v)| pi * (v - mean) * (v - mean)).sum();
    (mean, var)
}

/// Sensitivity of `target` to findings at each of `candidates`, given the
/// evidence currently entered in `engine`. The engine's evidence is restored
/// before returning.
pub fn sensitivity_to_findings(
    engine: &mut Engine,
    net: &Network,
    target: NodeId,
    candidates: &[NodeId],
) -> Result<Vec<SensRow>, InferenceError> {
    let saved = engine.evidence().clone();
    let base = engine.beliefs(target)?;
    let h_base = entropy_bits(&base);
    let values: Option<Vec<f64>> =
        net.node(target).states.iter().map(|s| s.value).collect();
    let var_base = values.as_ref().map(|v| mean_var(&base, v).1);

    let mut out = Vec::new();
    for &cand in candidates {
        if cand == target {
            continue;
        }
        let p_cand = match engine.beliefs(cand) {
            Ok(p) => p,
            Err(InferenceError::NotAVariable) => continue,
            Err(e) => {
                engine.set_evidence(saved.clone());
                return Err(e);
            }
        };
        let mut mi = 0.0;
        let mut exp_var = 0.0;
        for (s, &pf) in p_cand.iter().enumerate() {
            if pf <= 0.0 {
                continue;
            }
            engine.set_evidence(saved.clone());
            engine.set_finding(cand, Finding::Hard(s))?;
            let post = match engine.beliefs(target) {
                Ok(p) => p,
                Err(InferenceError::ConflictingEvidence) => continue,
                Err(e) => {
                    engine.set_evidence(saved);
                    return Err(e);
                }
            };
            for (t, &pt) in post.iter().enumerate() {
                if pt > 0.0 && base[t] > 0.0 {
                    mi += pf * pt * (pt / base[t]).log2();
                }
            }
            if let Some(v) = &values {
                exp_var += pf * mean_var(&post, v).1;
            }
        }
        out.push(SensRow {
            node: cand,
            mutual_info: mi.max(0.0),
            entropy_reduction_pct: if h_base > 0.0 { 100.0 * mi.max(0.0) / h_base } else { 0.0 },
            variance_reduction: var_base.map(|vb| vb - exp_var),
        });
    }
    engine.set_evidence(saved);
    out.sort_by(|a, b| b.mutual_info.partial_cmp(&a.mutual_info).unwrap());
    Ok(out)
}
