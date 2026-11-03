//! AI settings persisted as JSON at `~/.config/balina/settings.json` (all
//! platforms — deliberately not `dirs::config_dir()`). API keys are stored in
//! cleartext with owner-only file permissions, the `~/.aws/credentials` trust
//! model.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Serde spellings must stay identical to [`ProviderKind::id`] — the ids the
/// UI shows — so a hand-edited settings file round-trips (the aliases keep
/// files written by earlier builds, which used kebab-cased variant names,
/// loading).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ProviderKind {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai", alias = "open-ai")]
    OpenAi,
    /// Any OpenAI-compatible chat endpoint (Perplexity, local inference
    /// servers) addressed by its full chat-completions URL.
    #[serde(rename = "openai-compatible", alias = "open-ai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl ProviderKind {
    /// Text-role providers — all generative. The structured role runs local
    /// Laya and has no provider choice.
    pub const ALL: [ProviderKind; 4] = [
        ProviderKind::Anthropic,
        ProviderKind::OpenAi,
        ProviderKind::OpenAiCompatible,
        ProviderKind::OpenRouter,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::OpenAiCompatible => "openai-compatible",
            ProviderKind::OpenRouter => "openrouter",
        }
    }

    pub fn from_id(id: &str) -> Option<ProviderKind> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::OpenAiCompatible => "OpenAI-compatible (Perplexity, local, …)",
            ProviderKind::OpenRouter => "OpenRouter",
        }
    }

    /// URL the provider starts from when selected — the FULL chat-completions
    /// URL. Empty = the field stays hidden.
    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::OpenRouter => "https://openrouter.ai/api/v1/chat/completions",
            _ => "",
        }
    }

    pub fn shows_base_url(self) -> bool {
        matches!(self, ProviderKind::OpenAiCompatible | ProviderKind::OpenRouter)
    }

    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "claude-sonnet-5",
            ProviderKind::OpenAi => "gpt-5",
            ProviderKind::OpenAiCompatible => "",
            ProviderKind::OpenRouter => "anthropic/claude-sonnet-5",
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct RoleConfig {
    pub provider: ProviderKind,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
}

impl RoleConfig {
    pub fn for_provider(provider: ProviderKind) -> Self {
        RoleConfig {
            provider,
            base_url: provider.default_base_url().to_string(),
            api_key: String::new(),
            model: provider.default_model().to_string(),
        }
    }
}

/// Structured role: local Laya inference — no provider, key, or URL, just
/// which checkpoint to run.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct StructuredSettings {
    #[serde(default)]
    pub checkpoint: crate::ai::laya_local::LayaCheckpoint,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AiSettings {
    /// Generative role: network drafting, explanations.
    pub text: RoleConfig,
    /// Structured role: state probabilities, CPT rows — local Laya.
    pub structured: StructuredSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            text: RoleConfig::for_provider(ProviderKind::Anthropic),
            structured: StructuredSettings::default(),
        }
    }
}

/// Top-level settings file shape; `ai` is one key so future non-AI settings
/// can share the file without a migration. Fields are parsed independently
/// and leniently: an older file may hold shapes that no longer exist (e.g. a
/// hosted-provider structured role), and must degrade to defaults, not fail.
#[derive(Clone, Default, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    ai: Option<AiFileSection>,
}

#[derive(Clone, Default, Deserialize)]
struct AiFileSection {
    #[serde(default)]
    text: Option<serde_json::Value>,
    #[serde(default)]
    structured: Option<serde_json::Value>,
}

pub fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config").join("balina").join("settings.json"))
}

/// Missing or unreadable file yields defaults; a parse error must not take
/// the dialog down with it.
pub fn load() -> AiSettings {
    settings_path().map(|p| load_from(&p)).unwrap_or_default()
}

fn load_from(path: &Path) -> AiSettings {
    let section = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<SettingsFile>(&text).ok())
        .and_then(|f| f.ai)
        .unwrap_or_default();
    fn lenient<T: serde::de::DeserializeOwned>(v: Option<serde_json::Value>) -> Option<T> {
        v.and_then(|v| serde_json::from_value(v).ok())
    }
    // Full parse, else field-by-field salvage: one malformed field must not
    // reset the whole role — the next Save would silently wipe the stored
    // API key.
    fn lenient_role(v: Option<serde_json::Value>) -> Option<RoleConfig> {
        let v = v?;
        if let Ok(cfg) = serde_json::from_value::<RoleConfig>(v.clone()) {
            return Some(cfg);
        }
        let obj = v.as_object()?;
        let provider = serde_json::from_value(obj.get("provider")?.clone()).ok()?;
        let field = |k: &str| {
            obj.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
        };
        Some(RoleConfig {
            provider,
            base_url: field("base_url"),
            api_key: field("api_key"),
            model: field("model"),
        })
    }
    let defaults = AiSettings::default();
    AiSettings {
        text: lenient_role(section.text).unwrap_or(defaults.text),
        structured: lenient(section.structured).unwrap_or(defaults.structured),
    }
}

pub fn save(settings: &AiSettings) -> Result<(), String> {
    let path = settings_path().ok_or("no home directory")?;
    save_to(settings, &path)
}

fn save_to(settings: &AiSettings, path: &Path) -> Result<(), String> {
    // Preserve unrelated top-level keys a future version may have written.
    let mut root: serde_json::Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    root["ai"] = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("balina-ai-settings-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn round_trip() {
        let path = tmp_file("round-trip");
        let mut s = AiSettings::default();
        s.text.api_key = "sk-test".into();
        s.text.provider = ProviderKind::OpenRouter;
        s.structured.checkpoint = crate::ai::laya_local::LayaCheckpoint::TypedDecisions;
        save_to(&s, &path).unwrap();
        assert_eq!(load_from(&path), s);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn old_hosted_structured_role_degrades_to_default() {
        // A settings file from before local Laya: structured was a hosted
        // RoleConfig. It must not break the file — text survives, structured
        // falls back to the default checkpoint.
        let path = tmp_file("migrate");
        std::fs::write(
            &path,
            r#"{"ai": {
                "text": {"provider": "anthropic", "api_key": "sk-keep", "model": "claude-sonnet-5"},
                "structured": {"provider": "laya", "base_url": "https://api.laya-ai.com", "api_key": "lk", "model": "english"}
            }}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.text.api_key, "sk-keep");
        assert_eq!(s.structured, StructuredSettings::default());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn missing_or_corrupt_file_yields_defaults() {
        let path = tmp_file("corrupt");
        assert_eq!(load_from(&path), AiSettings::default());
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_from(&path), AiSettings::default());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_preserves_foreign_keys() {
        let path = tmp_file("foreign");
        std::fs::write(&path, r#"{"theme": "dark"}"#).unwrap();
        save_to(&AiSettings::default(), &path).unwrap();
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["theme"], "dark");
        assert!(root["ai"].is_object());
        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_file("perms");
        save_to(&AiSettings::default(), &path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn provider_ids_round_trip() {
        for p in ProviderKind::ALL {
            assert_eq!(ProviderKind::from_id(p.id()), Some(p));
        }
    }

    #[test]
    fn provider_serde_spelling_matches_ui_id() {
        // A hand-edited file using the id the app displays must load; a
        // mismatch here silently reset the role and wiped the API key.
        for p in ProviderKind::ALL {
            assert_eq!(serde_json::to_value(p).unwrap(), serde_json::json!(p.id()));
            assert_eq!(
                serde_json::from_value::<ProviderKind>(serde_json::json!(p.id())).unwrap(),
                p
            );
        }
    }

    #[test]
    fn legacy_kebab_provider_spellings_still_load() {
        // Earlier builds serialized variant names via kebab-case.
        for (old, expect) in [
            ("open-ai", ProviderKind::OpenAi),
            ("open-ai-compatible", ProviderKind::OpenAiCompatible),
        ] {
            assert_eq!(
                serde_json::from_value::<ProviderKind>(serde_json::json!(old)).unwrap(),
                expect
            );
        }
    }

    #[test]
    fn hand_edited_ui_spelling_keeps_api_key() {
        let path = tmp_file("ui-spelling");
        std::fs::write(
            &path,
            r#"{"ai": {"text": {"provider": "openai", "api_key": "sk-keep", "model": "gpt-5"}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.text.provider, ProviderKind::OpenAi);
        assert_eq!(s.text.api_key, "sk-keep");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn malformed_field_does_not_wipe_the_key() {
        // `model` has the wrong type; the salvage path keeps the rest.
        let path = tmp_file("salvage");
        std::fs::write(
            &path,
            r#"{"ai": {"text": {"provider": "anthropic", "api_key": "sk-keep", "model": 5}}}"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.text.provider, ProviderKind::Anthropic);
        assert_eq!(s.text.api_key, "sk-keep");
        assert_eq!(s.text.model, "");
        std::fs::remove_file(&path).unwrap();
    }
}
