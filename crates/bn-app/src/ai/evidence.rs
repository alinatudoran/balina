//! Findings from free text through local Laya: one choice question per
//! chance node — its states plus an explicit "no information" option — in a
//! single batched forward pass. Replies are validated offline
//! (`bn_session::ai_evidence`); results feed a review list — nothing becomes
//! evidence until the user applies it.

use bn_session::ai_evidence;

use super::laya_local::{self, LayaError};
use super::settings::StructuredSettings;

/// The abstain option appended to every question; distinctive enough never
/// to collide with a real state name.
const ABSTAIN: &str = "(the text gives no information)";

/// One chance node the model is asked about.
#[derive(Clone, PartialEq)]
pub struct NodeQuery {
    pub name: String,
    pub title: String,
    pub comment: String,
    pub states: Vec<String>,
}

impl NodeQuery {
    fn description(&self) -> String {
        let mut d = format!("`{}`", self.name);
        if !self.title.is_empty() {
            d.push_str(&format!(" ({})", self.title));
        }
        if !self.comment.is_empty() {
            d.push_str(&format!(": {}", self.comment));
        }
        d
    }
}

/// Ask local Laya what the text says about each node. Returns
/// `(node name, normalized distribution)` for nodes with information; a node
/// abstains when the "no information" option wins its question.
pub async fn suggest_evidence(
    cfg: StructuredSettings,
    nodes: Vec<NodeQuery>,
    text: String,
) -> Result<Vec<(String, Vec<f64>)>, LayaError> {
    if nodes.is_empty() {
        return Err(LayaError::Other(
            "The network has no chance nodes to enter findings on.".into(),
        ));
    }
    if nodes.len() > ai_evidence::MAX_EVIDENCE_NODES {
        return Err(LayaError::Other(format!(
            "The network has {} chance nodes (limit {} per extraction).",
            nodes.len(),
            ai_evidence::MAX_EVIDENCE_NODES
        )));
    }
    if text.trim().is_empty() {
        return Err(LayaError::Other("The text is empty.".into()));
    }

    let questions: Vec<(String, laya::Question)> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let options =
                n.states.iter().cloned().chain(std::iter::once(ABSTAIN.to_string()));
            (
                format!("q{i}"),
                laya::Question::choice(
                    &format!("What does the text say about {}?", n.description()),
                    options,
                ),
            )
        })
        .collect();

    let response = laya_local::run_questions(cfg.checkpoint, text, questions).await?;

    let mut parsed: Vec<ai_evidence::ParsedFinding> = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let mut options = n.states.clone();
        options.push(ABSTAIN.to_string());
        let probs = laya_local::choice_distribution(&response, &format!("q{i}"), &options)
            .map_err(LayaError::Other)?;
        let (none, states) = (probs[n.states.len()], &probs[..n.states.len()]);
        let abstain = states.iter().all(|&p| none >= p) || states.iter().sum::<f64>() <= 0.0;
        parsed.push((n.name.clone(), (!abstain).then(|| states.to_vec())));
    }

    let shapes: Vec<(String, usize)> =
        nodes.iter().map(|n| (n.name.clone(), n.states.len())).collect();
    ai_evidence::validate_findings(parsed, &shapes).map_err(|problems| {
        LayaError::Other(format!("Laya findings invalid:\n• {}", problems.join("\n• ")))
    })
}
