//! LLM provider round-trips for the text role: model construction and
//! `test_connection`, a minimal call proving endpoint + key + model are
//! usable. (The structured role is local Laya — see `laya_local`.)

use ai::{Context, Message, StreamOptions, complete};

use super::settings::{ProviderKind, RoleConfig};

const TIMEOUT_MS: u64 = 30_000;

/// One cheap authenticated request; `Ok` carries a user-facing summary and
/// `Err` the provider's complaint. Runs off the UI thread via `spawn`.
pub async fn test_connection(cfg: RoleConfig) -> Result<String, String> {
    test_llm(cfg).await
}

/// The `ai` crate stores an API base and appends `/chat/completions` itself,
/// while the settings field holds the FULL endpoint URL for OpenAI-compatible
/// and OpenRouter providers — strip the suffix back off. A plain base URL
/// passes through unchanged.
fn completions_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix("/chat/completions").unwrap_or(url).trim_end_matches('/').to_string()
}

/// Build an executable LLM model handle from a text-role config.
pub fn build_llm_model(cfg: &RoleConfig) -> Result<ai::Model, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("API key is empty.".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("Model name is empty.".into());
    }
    let err = |e: ai::Error| e.to_string();
    match cfg.provider {
        ProviderKind::Anthropic => {
            let mut b = ai::providers::anthropic::builder().api_key(cfg.api_key.trim());
            if !cfg.base_url.trim().is_empty() {
                b = b.base_url(cfg.base_url.trim());
            }
            b.build().map_err(err)?.model(cfg.model.trim()).build().map_err(err)
        }
        ProviderKind::OpenAi => ai::providers::openai::builder()
            .api_key(Some(cfg.api_key.trim()))
            .chat_completions()
            .build()
            .map_err(err)?
            .model(cfg.model.trim())
            .build()
            .map_err(err),
        ProviderKind::OpenAiCompatible => {
            if cfg.base_url.trim().is_empty() {
                return Err("URL is required for an OpenAI-compatible provider.".into());
            }
            ai::providers::openai::builder()
                .provider_id("openai-compatible")
                .api_key(Some(cfg.api_key.trim()))
                .base_url(completions_base(&cfg.base_url))
                .chat_completions()
                .build()
                .map_err(err)?
                .model(cfg.model.trim())
                .build()
                .map_err(err)
        }
        ProviderKind::OpenRouter => {
            let url = if cfg.base_url.trim().is_empty() {
                ProviderKind::OpenRouter.default_base_url().to_string()
            } else {
                cfg.base_url.clone()
            };
            ai::providers::openai::builder()
                .provider_id("openrouter")
                .api_key(Some(cfg.api_key.trim()))
                .base_url(completions_base(&url))
                .chat_completions()
                .build()
                .map_err(err)?
                .model(cfg.model.trim())
                .build()
                .map_err(err)
        }
    }
}

/// Concatenated text blocks of a reply (thinking/tool blocks skipped).
pub fn text_of(msg: &ai::AssistantMessage) -> String {
    msg.content
        .iter()
        .filter_map(|c| match c {
            ai::AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect()
}

async fn test_llm(cfg: RoleConfig) -> Result<String, String> {
    let model = build_llm_model(&cfg)?;
    let context = Context::builder()
        .message(Message::user_text("Reply with the single word: ok"))
        .build();
    let options = StreamOptions {
        max_tokens: Some(32),
        timeout_ms: Some(TIMEOUT_MS),
        ..Default::default()
    };
    let msg = complete(model, context, Some(options)).await.map_err(|e| e.to_string())?;
    if let Some(e) = msg.error_message {
        return Err(e);
    }
    let replied = msg.response_model.unwrap_or(msg.model);
    Ok(format!("Connection OK — {replied} replied."))
}

#[cfg(test)]
mod tests {
    use super::completions_base;

    #[test]
    fn full_url_maps_to_crate_base() {
        assert_eq!(
            completions_base("https://openrouter.ai/api/v1/chat/completions"),
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(
            completions_base(" https://api.perplexity.ai/chat/completions/ "),
            "https://api.perplexity.ai"
        );
        // A plain base URL passes through unchanged.
        assert_eq!(completions_base("http://localhost:4141/v1/"), "http://localhost:4141/v1");
    }
}
