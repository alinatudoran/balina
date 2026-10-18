//! Draft a network from a plain-language description: one text-model call,
//! offline parse + validation (`bn_session::ai_draft`), and at most one
//! repair round-trip feeding the validation errors back to the model. The
//! result is a ready `Network`; the caller opens it as a new unsaved tab.

use ai::{Context, Message, StreamOptions, complete};
use bn_core::model::Network;
use bn_session::ai_draft;

use super::client::{build_llm_model, text_of};
use super::settings::RoleConfig;

const SYSTEM_PROMPT: &str = r#"You design Bayesian networks and influence diagrams. Turn the user's description or document into a draft network, and reply with ONE JSON object only — no prose, no code fence:

{
  "name": "short network name",
  "nodes": [
    {
      "name": "ShortId",
      "title": "Readable label (optional)",
      "kind": "chance",
      "states": ["state1", "state2"],
      "parents": ["NameOfCause"],
      "comment": "what this variable means (optional)"
    }
  ]
}

Rules:
- "kind" is "chance" (default), "decision", or "utility". Use decision/utility only when the text clearly describes choices and payoffs.
- Every chance/decision node needs 2-5 mutually exclusive states; utility nodes have none.
- "parents" lists names of direct causes/influences already in "nodes"; the graph must be acyclic; utility nodes cannot be parents.
- Node names must be unique, short identifiers; put longer descriptions in "title"/"comment".
- Model only variables and dependencies grounded in the text. Do not invent probabilities — CPTs are filled later."#;

fn request_options() -> StreamOptions {
    StreamOptions {
        max_tokens: Some(8192),
        timeout_ms: Some(180_000),
        ..Default::default()
    }
}

fn parse_and_build(reply: &str) -> Result<Network, Vec<String>> {
    let draft = ai_draft::parse_draft(reply).map_err(|e| vec![e])?;
    ai_draft::build_network(&draft)
}

/// One draft call plus one self-repair round-trip; `Err` is user-readable.
pub async fn draft_network(cfg: RoleConfig, description: String) -> Result<Network, String> {
    let model = build_llm_model(&cfg)?;
    let context = Context::builder()
        .system_prompt(SYSTEM_PROMPT)
        .message(Message::user_text(&description))
        .build();
    let first = complete(model.clone(), context, Some(request_options()))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(e) = &first.error_message {
        return Err(e.clone());
    }
    let problems = match parse_and_build(&text_of(&first)) {
        Ok(net) => return Ok(net),
        Err(problems) => problems,
    };

    // Repair round-trip: the invalid draft stays in context, the validator's
    // complaints become the follow-up message.
    let feedback = format!(
        "That draft failed validation:\n- {}\nReturn the corrected JSON object only.",
        problems.join("\n- ")
    );
    let retry_context = Context::builder()
        .system_prompt(SYSTEM_PROMPT)
        .message(Message::user_text(&description))
        .message(Message::Assistant(first))
        .message(Message::user_text(feedback))
        .build();
    let second = complete(model, retry_context, Some(request_options()))
        .await
        .map_err(|e| e.to_string())?;
    if let Some(e) = &second.error_message {
        return Err(e.clone());
    }
    parse_and_build(&text_of(&second)).map_err(|problems| {
        format!(
            "The draft still failed validation after a retry:\n• {}",
            problems.join("\n• ")
        )
    })
}
