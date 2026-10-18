//! Evidence ops (Dirt::Evidence).

use bn_core::model::NodeId;

use crate::error::CmdError;
use crate::session::Session;
use crate::views::check_node;

/// Click-again-retracts semantics preserved from the egui canvas.
pub fn toggle_finding(s: &mut Session, node: NodeId, state: usize) -> Result<(), CmdError> {
    check_node(&s.doc().net, node)?;
    if state >= s.doc().net.node(node).n_states() {
        return Err(CmdError::BadRequest(format!("state index {state} out of range")));
    }
    let dirt = s.doc_mut().toggle_finding(node, state);
    s.finish(dirt);
    Ok(())
}

pub fn set_likelihood_finding(
    s: &mut Session,
    node: NodeId,
    likelihood: Vec<f64>,
) -> Result<(), CmdError> {
    check_node(&s.doc().net, node)?;
    let card = s.doc().net.node(node).n_states();
    if likelihood.len() != card {
        return Err(CmdError::BadRequest(format!(
            "likelihood needs {card} values, got {}",
            likelihood.len()
        )));
    }
    if likelihood.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(CmdError::BadRequest("likelihood values must be finite and ≥ 0".into()));
    }
    let dirt = s.doc_mut().set_likelihood_finding(node, likelihood);
    s.finish(dirt);
    Ok(())
}

/// Apply several likelihood findings as ONE undoable step (accepting
/// AI-extracted text evidence). All-or-nothing: every entry is validated
/// before any is applied.
pub fn set_likelihood_findings(
    s: &mut Session,
    findings: Vec<(NodeId, Vec<f64>)>,
) -> Result<(), CmdError> {
    if findings.is_empty() {
        return Err(CmdError::BadRequest("no findings to apply".into()));
    }
    for (node, likelihood) in &findings {
        check_node(&s.doc().net, *node)?;
        let name = &s.doc().net.node(*node).name;
        let card = s.doc().net.node(*node).n_states();
        if likelihood.len() != card {
            return Err(CmdError::BadRequest(format!(
                "`{name}`: likelihood needs {card} values, got {}",
                likelihood.len()
            )));
        }
        if likelihood.iter().any(|v| !v.is_finite() || *v < 0.0)
            || !likelihood.iter().any(|v| *v > 0.0)
        {
            return Err(CmdError::BadRequest(format!(
                "`{name}`: likelihood values must be finite, ≥ 0, with at least one > 0"
            )));
        }
    }
    s.doc_mut().begin_change();
    for (node, likelihood) in findings {
        s.doc_mut().evidence.set(node, bn_core::inference::Finding::Likelihood(likelihood));
    }
    s.finish(crate::doc::Dirt::Evidence);
    Ok(())
}

pub fn retract_finding(s: &mut Session, node: NodeId) -> Result<(), CmdError> {
    check_node(&s.doc().net, node)?;
    let dirt = s.doc_mut().retract_finding(node);
    s.finish(dirt);
    Ok(())
}

pub fn retract_all_findings(s: &mut Session) {
    let dirt = s.doc_mut().retract_all_findings();
    s.finish(dirt);
}
