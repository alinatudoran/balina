//! Definite findings: an optional text-model pass after local Laya. Laya's
//! choice distributions stay below 1 even when the text states a fact
//! outright ("non-smoker" → ~0.95); the text model names the nodes whose
//! state the text gives unambiguously, so the review can snap them to hard
//! findings. Offline parse + resolution live in `bn_session::ai_evidence`.

use ai::{Context, Message, StreamOptions, complete};
use bn_session::ai_evidence;

use super::client::{build_llm_model, text_of};
use super::evidence::NodeQuery;
use super::settings::RoleConfig;

const SYSTEM_PROMPT: &str = r#"You check findings extracted from a case note. For each listed variable, decide whether the text states its value EXPLICITLY and UNAMBIGUOUSLY — e.g. "non-smoker", "has not visited Asia", "X-ray is positive". Hedged, suspected, inferred or probabilistic language ("likely", "possible", "suspected", "may have", "risk of") is NOT definite. Use only what the text says, never domain knowledge.

Reply with ONE JSON object only — no prose, no code fence:

{"definite": [{"node": "NodeName", "state": "exact state name"}]}

List only the definite variables, using the node and state names exactly as given; use an empty list when none is definite."#;

fn request_options() -> StreamOptions {
    StreamOptions {
        max_tokens: Some(2048),
        timeout_ms: Some(60_000),
        ..Default::default()
    }
}

/// Ask the text model which suggested findings the text states outright.
/// `nodes` pairs each queried node with Laya's distribution (context only).
/// Returns `(node name, state index)`; `Err` is user-readable.
pub async fn detect_definite(
    cfg: RoleConfig,
    text: String,
    nodes: Vec<(NodeQuery, Vec<f64>)>,
) -> Result<Vec<(String, usize)>, String> {
    let model = build_llm_model(&cfg)?;
    let mut msg = format!("Text:\n\"\"\"\n{}\n\"\"\"\n\nVariables:\n", text.trim());
    for (n, probs) in &nodes {
        let states: Vec<String> = n
            .states
            .iter()
            .zip(probs)
            .map(|(s, p)| format!("\"{s}\" (suggested {p:.2})"))
            .collect();
        msg.push_str(&format!("- {} — states: {}\n", n.description(), states.join(", ")));
    }
    let context = Context::builder()
        .system_prompt(SYSTEM_PROMPT)
        .message(Message::user_text(&msg))
        .build();
    let reply = complete(model, context, Some(request_options()))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(e) = reply.error_message {
        return Err(e);
    }
    let pairs = ai_evidence::parse_definite_reply(&text_of(&reply))?;
    let shapes: Vec<(String, Vec<String>)> =
        nodes.iter().map(|(n, _)| (n.name.clone(), n.states.clone())).collect();
    Ok(ai_evidence::resolve_definite(pairs, &shapes))
}
