//! Desktop implementations: rfd native dialogs, std::fs, tokio blocking pool.

use std::path::PathBuf;

use bn_session::jobs::{JobCtx, JobEvent};
use bn_session::ops::learn::{StructureJobInput, StructureOutcome};
use bn_session::views::StructureLearnResult;
use bn_session::{CmdError, Session};
use dioxus::prelude::*;

use crate::state::{JOB_PROGRESS, LAST_CASE_FILE};

/// A picked case file: a plain path on desktop.
#[derive(Clone, Debug)]
pub struct CaseFile {
    path: PathBuf,
}

impl CaseFile {
    /// File name for dialog labels.
    pub fn label(&self) -> String {
        self.path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn forced_delim(&self) -> Option<u8> {
        bn_session::ops::learn::delim_for_name(&self.path.to_string_lossy())
    }

    pub async fn read(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.path).map_err(|e| format!("cannot open file: {e}"))
    }
}

/// Pick a CSV/TSV case file; remembers the pick for the next dialog.
pub async fn pick_case_file() -> Option<CaseFile> {
    let mut d = rfd::AsyncFileDialog::new().add_filter("Case files", &["csv", "tsv"]);
    if let Some(last) = LAST_CASE_FILE.peek().as_ref().and_then(|f| f.path.parent()) {
        d = d.set_directory(last);
    }
    let fh = d.pick_file().await?;
    let picked = CaseFile { path: fh.path().to_path_buf() };
    *LAST_CASE_FILE.write() = Some(picked.clone());
    Some(picked)
}

pub async fn sleep_ms(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

pub fn set_window_title(title: &str) {
    dioxus::desktop::window().set_title(title);
}

/// Native warning dialog with OK/Cancel; true = OK.
pub async fn confirm(title: &str, message: &str) -> bool {
    let choice = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show()
        .await;
    matches!(choice, rfd::MessageDialogResult::Ok)
}

/// Run a prepared structure-learning job off the UI thread, streaming
/// progress into `JOB_PROGRESS`. `Err` = the blocking task itself crashed.
pub async fn run_structure_job(
    input: StructureJobInput,
    cases: CaseFile,
) -> Result<StructureOutcome, String> {
    let cancel = input.cancel.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<JobEvent>();
    let jc = JobCtx::new(cancel, move |e| {
        let _ = tx.send(e);
    });
    let handle = tokio::task::spawn_blocking(move || {
        // Read inside the blocking task; same failure text as when the job
        // body did the open itself.
        let bytes = match std::fs::read(&cases.path) {
            Ok(b) => b,
            Err(e) => return StructureOutcome::Failed(format!("cannot open file: {e}")),
        };
        bn_session::ops::learn::run_structure_job(input, &bytes, cases.forced_delim(), &jc)
    });
    // Drain progress on the UI scheduler.
    spawn(async move {
        while let Some(e) = rx.recv().await {
            *JOB_PROGRESS.write() = Some(e);
        }
    });
    handle.await.map_err(|e| e.to_string())
}

/// Desktop applies the outcome by clone-swap (NodeIds are shared).
pub fn apply_structure_outcome(
    s: &mut Session,
    outcome: StructureOutcome,
    started_seq: u64,
) -> Result<StructureLearnResult, CmdError> {
    bn_session::ops::learn::apply_structure_outcome(s, outcome, started_seq)
}
