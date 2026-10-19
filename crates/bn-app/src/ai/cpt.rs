//! CPT elicitation through local Laya: one choice question per parent
//! configuration, answered in a single batched forward pass. Replies are
//! shape-validated offline (`bn_session::ai_cpt`); the result fills the CPT
//! editor's draft grid — nothing is applied until the user accepts.

use bn_session::ai_cpt;

use super::laya_local::{self, LayaError};
use super::settings::StructuredSettings;

/// Everything the model needs to know about the table being filled, in the
/// engine's row order.
pub struct CptPrompt {
    pub node_name: String,
    pub node_title: String,
    pub node_comment: String,
    /// Child states, CPT column order.
    pub states: Vec<String>,
    pub parent_names: Vec<String>,
    /// One label per CPT row: "Smoking=yes, Bronchitis=no" or "(no parents)".
    pub row_labels: Vec<String>,
    /// The user's verbal judgments, e.g. "cancer is very likely if both...".
    pub guidance: String,
}

impl CptPrompt {
    fn node_description(&self) -> String {
        let mut d = format!("Variable `{}`", self.node_name);
        if !self.node_title.is_empty() {
            d.push_str(&format!(" ({})", self.node_title));
        }
        if !self.node_comment.is_empty() {
            d.push_str(&format!(": {}", self.node_comment));
        }
        if !self.parent_names.is_empty() {
            d.push_str(&format!("; direct causes: {}", self.parent_names.join(", ")));
        }
        if !self.guidance.trim().is_empty() {
            d.push_str(&format!("\nDomain judgment from the user: {}", self.guidance.trim()));
        }
        d
    }
}

pub async fn suggest_cpt(
    cfg: StructuredSettings,
    prompt: CptPrompt,
) -> Result<Vec<f64>, LayaError> {
    if prompt.row_labels.len() > ai_cpt::MAX_CPT_ROWS {
        return Err(LayaError::Other(format!(
            "This table has {} parent configurations (limit {}). Fill it from data \
             (Cases > Learn CPTs) or reduce parents/states.",
            prompt.row_labels.len(),
            ai_cpt::MAX_CPT_ROWS
        )));
    }
    let questions: Vec<(String, laya::Question)> = prompt
        .row_labels
        .iter()
        .enumerate()
        .map(|(r, label)| {
            let instructions = if prompt.parent_names.is_empty() {
                format!("Which state of `{}` is most probable?", prompt.node_name)
            } else {
                format!("Given {label}, which state of `{}` is most probable?", prompt.node_name)
            };
            (
                format!("r{r}"),
                laya::Question::choice(&instructions, prompt.states.iter().cloned()),
            )
        })
        .collect();

    let response =
        laya_local::run_questions(cfg.checkpoint, prompt.node_description(), questions).await?;
    let mut rows = Vec::with_capacity(prompt.row_labels.len());
    for r in 0..prompt.row_labels.len() {
        rows.push(
            laya_local::choice_distribution(&response, &format!("r{r}"), &prompt.states)
                .map_err(LayaError::Other)?,
        );
    }
    ai_cpt::validate_rows(&rows, prompt.row_labels.len(), prompt.states.len()).map_err(
        |problems| {
            LayaError::Other(format!("Laya probabilities invalid:\n• {}", problems.join("\n• ")))
        },
    )
}
