//! Local Laya inference (the structured model role). The `laya` crate runs
//! the open-weights checkpoint with candle — synchronous, so every load and
//! forward pass goes through `spawn_blocking`. The crate has no downloader;
//! [`download`] fetches the checkpoint files from Hugging Face into
//! `~/.config/balina/models/laya/<checkpoint>/` with progress + cancel.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

const HF_BASE: &str = "https://huggingface.co/convaiinnovations/laya/resolve/main";

/// The five files `laya::Agent::from_dir` expects, relative to the model dir.
/// On Hugging Face the non-English checkpoints live under a subfolder with
/// this same layout.
const FILES: [&str; 5] = [
    "model.safetensors",
    "encoder/config.json",
    "tokenizer/tokenizer.json",
    "tokenizer/tokenizer_config.json",
    "rl_agent_config.json",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayaCheckpoint {
    #[default]
    English,
    Multilingual,
    TypedDecisions,
}

impl LayaCheckpoint {
    pub const ALL: [LayaCheckpoint; 3] = [
        LayaCheckpoint::English,
        LayaCheckpoint::Multilingual,
        LayaCheckpoint::TypedDecisions,
    ];

    pub fn id(self) -> &'static str {
        match self {
            LayaCheckpoint::English => "english",
            LayaCheckpoint::Multilingual => "multilingual",
            LayaCheckpoint::TypedDecisions => "typed-decisions",
        }
    }

    pub fn from_id(id: &str) -> Option<LayaCheckpoint> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            LayaCheckpoint::English => "English (847 MB)",
            LayaCheckpoint::Multilingual => "Multilingual (679 MB)",
            LayaCheckpoint::TypedDecisions => "Typed decisions (847 MB)",
        }
    }

    /// Subfolder inside the HF bundle repo; the English checkpoint is the
    /// repo root.
    fn hf_prefix(self) -> &'static str {
        match self {
            LayaCheckpoint::English => "",
            LayaCheckpoint::Multilingual => "multilingual/",
            LayaCheckpoint::TypedDecisions => "typed-decisions/",
        }
    }

    /// Download size denominator for the progress bar (sizes from the HF
    /// tree; the three config files round to nothing).
    pub fn approx_total_bytes(self) -> u64 {
        match self {
            LayaCheckpoint::English => 846_200_000,
            LayaCheckpoint::Multilingual => 678_200_000,
            LayaCheckpoint::TypedDecisions => 846_200_000,
        }
    }

    /// `(remote URL, local path)` for every checkpoint file.
    pub fn file_plan(self) -> Vec<(String, PathBuf)> {
        let dir = self.dir();
        FILES
            .iter()
            .map(|f| (format!("{HF_BASE}/{}{f}", self.hf_prefix()), dir.join(f)))
            .collect()
    }

    pub fn dir(self) -> PathBuf {
        models_root().join(self.id())
    }

    pub fn is_installed(self) -> bool {
        self.file_plan()
            .iter()
            .all(|(_, p)| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
    }
}

fn models_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("balina")
        .join("models")
        .join("laya")
}

#[derive(Debug, Clone)]
pub enum LayaError {
    /// Checkpoint files are missing — the caller should offer the download.
    NotInstalled(LayaCheckpoint),
    Cancelled,
    Other(String),
}

impl std::fmt::Display for LayaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayaError::NotInstalled(cp) => {
                write!(f, "The Laya {} model is not installed.", cp.id())
            }
            LayaError::Cancelled => write!(f, "Download cancelled."),
            LayaError::Other(e) => write!(f, "{e}"),
        }
    }
}

// ---- download ---------------------------------------------------------------

/// Fetch every checkpoint file, streaming to `<file>.part` and renaming on
/// completion so an interrupted download never looks installed. `progress`
/// gets `(bytes downloaded, approx total)`; `cancel` is polled per chunk.
/// Runs on the caller's task — safe to update UI signals from `progress`.
pub async fn download(
    cp: LayaCheckpoint,
    progress: impl Fn(u64, u64),
    cancel: Arc<AtomicBool>,
) -> Result<(), LayaError> {
    let total = cp.approx_total_bytes();
    let client = reqwest::Client::new();
    let mut downloaded: u64 = 0;
    for (url, path) in cp.file_plan() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LayaError::Other(e.to_string()))?;
        }
        let part = path.with_extension("part");
        let result = fetch_one(&client, &url, &part, &mut downloaded, total, &progress, &cancel)
            .await;
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        std::fs::rename(&part, &path).map_err(|e| LayaError::Other(e.to_string()))?;
    }
    // A fresh download invalidates any cached agent of this checkpoint.
    if let Ok(mut slot) = AGENT.lock()
        && slot.as_ref().is_some_and(|(c, _)| *c == cp)
    {
        *slot = None;
    }
    Ok(())
}

async fn fetch_one(
    client: &reqwest::Client,
    url: &str,
    part: &std::path::Path,
    downloaded: &mut u64,
    total: u64,
    progress: &impl Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), LayaError> {
    use std::io::Write;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| LayaError::Other(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(LayaError::Other(format!("HTTP {} for {url}", resp.status())));
    }
    let mut file = std::fs::File::create(part).map_err(|e| LayaError::Other(e.to_string()))?;
    while let Some(chunk) = resp.chunk().await.map_err(|e| LayaError::Other(e.to_string()))? {
        if cancel.load(Ordering::Relaxed) {
            return Err(LayaError::Cancelled);
        }
        file.write_all(&chunk).map_err(|e| LayaError::Other(e.to_string()))?;
        *downloaded += chunk.len() as u64;
        progress((*downloaded).min(total), total);
    }
    file.flush().map_err(|e| LayaError::Other(e.to_string()))?;
    Ok(())
}

// ---- inference --------------------------------------------------------------

/// One loaded agent at a time (~1.3–1.7 GB resident); switching checkpoints
/// drops the previous one. The lock is never held across an await.
static AGENT: Mutex<Option<(LayaCheckpoint, Arc<laya::Agent>)>> = Mutex::new(None);

async fn get_agent(cp: LayaCheckpoint) -> Result<Arc<laya::Agent>, LayaError> {
    if let Some(agent) = AGENT
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().filter(|(c, _)| *c == cp).map(|(_, a)| a.clone()))
    {
        return Ok(agent);
    }
    if !cp.is_installed() {
        return Err(LayaError::NotInstalled(cp));
    }
    let dir = cp.dir();
    let agent = tokio::task::spawn_blocking(move || {
        laya::Agent::from_dir(dir, laya::Options::default()).map(Arc::new)
    })
    .await
    .map_err(|e| LayaError::Other(e.to_string()))?
    .map_err(|e| LayaError::Other(format!("could not load the Laya model: {e}")))?;
    if let Ok(mut slot) = AGENT.lock() {
        *slot = Some((cp, agent.clone()));
    }
    Ok(agent)
}

/// Answer `questions` about `state` in one batched forward pass (loading the
/// model first if needed — the cold load takes several seconds).
pub async fn run_questions(
    cp: LayaCheckpoint,
    state: String,
    questions: Vec<(String, laya::Question)>,
) -> Result<laya::Response, LayaError> {
    let agent = get_agent(cp).await?;
    tokio::task::spawn_blocking(move || {
        agent.system_one(&serde_json::Value::String(state), &questions)
    })
    .await
    .map_err(|e| LayaError::Other(e.to_string()))?
    .map_err(|e| LayaError::Other(format!("Laya inference failed: {e}")))
}

/// The structured-model "Test" button: a one-question round trip.
pub async fn test_local(cp: LayaCheckpoint) -> Result<String, String> {
    let start = std::time::Instant::now();
    let questions =
        vec![("ping".to_string(), laya::Question::noul("This is a connection test."))];
    match run_questions(cp, "connection test".into(), questions).await {
        Ok(_) => Ok(format!(
            "Local Laya OK — {} answered in {:.1} s.",
            cp.id(),
            start.elapsed().as_secs_f64()
        )),
        Err(e) => Err(e.to_string()),
    }
}

// ---- answer mapping ---------------------------------------------------------

/// Pull a choice answer's distribution out of a [`laya::Response`] in our
/// option order. The crate keys `probabilities` by the option strings; a
/// missing key reads as 0.
pub fn choice_distribution(
    response: &laya::Response,
    question_id: &str,
    options: &[String],
) -> Result<Vec<f64>, String> {
    let answer = response
        .answers
        .get(question_id)
        .ok_or_else(|| format!("Laya reply missing answer `{question_id}`"))?;
    let probs = answer
        .get("probabilities")
        .and_then(|p| p.as_object())
        .ok_or_else(|| format!("answer `{question_id}` has no probabilities"))?;
    Ok(options
        .iter()
        .map(|o| probs.get(o).and_then(|v| v.as_f64()).unwrap_or(0.0))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_plan_covers_all_variants() {
        for cp in LayaCheckpoint::ALL {
            let plan = cp.file_plan();
            assert_eq!(plan.len(), 5);
            for (url, path) in &plan {
                assert!(url.starts_with(HF_BASE), "{url}");
                assert!(path.starts_with(cp.dir()));
            }
        }
        // English is the repo root; the others are subfolders.
        assert_eq!(
            LayaCheckpoint::English.file_plan()[0].0,
            format!("{HF_BASE}/model.safetensors")
        );
        assert_eq!(
            LayaCheckpoint::TypedDecisions.file_plan()[0].0,
            format!("{HF_BASE}/typed-decisions/model.safetensors")
        );
    }

    #[test]
    fn checkpoint_ids_round_trip() {
        for cp in LayaCheckpoint::ALL {
            assert_eq!(LayaCheckpoint::from_id(cp.id()), Some(cp));
        }
    }

    #[test]
    fn installed_requires_all_files_nonempty() {
        // Not installed against the real (likely absent) dir must not panic.
        let _ = LayaCheckpoint::English.is_installed();
    }

    #[test]
    fn choice_distribution_reads_by_option_name() {
        let response = laya::Response {
            model: "laya".into(),
            answers: serde_json::json!({
                "q0": {
                    "type": "choice",
                    "choice": "yes",
                    "probabilities": { "yes": 0.8, "no": 0.15, "maybe": 0.05 },
                    "confidence": 0.7,
                    "rl_agent": { "act_probability": 1.0 }
                }
            })
            .as_object()
            .unwrap()
            .clone(),
            usage: laya::Usage { input_tokens: 0, output_tokens: 0 },
        };
        let d =
            choice_distribution(&response, "q0", &["no".into(), "yes".into(), "gone".into()])
                .unwrap();
        assert_eq!(d, vec![0.15, 0.8, 0.0]);
        assert!(choice_distribution(&response, "missing", &[]).is_err());
    }
}
