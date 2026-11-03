//! Download dialog for a local Laya checkpoint: explicit start, streamed
//! progress, cancel (partial files staged and cleaned up). Opened from AI
//! Settings and from any structured feature that finds the model missing.
//!
//! Deliberately NOT in the single `DIALOG` slot: it renders as an overlay on
//! top of whatever dialog asked for it, so that dialog's unsaved state (draft
//! CPT cells, a pasted case note) survives the detour. All state is global —
//! the transfer runs on a detached task (`spawn_forever`), so closing the
//! overlay mid-download still cancels cleanly and removes the staged files.

use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;

use crate::ai::laya_local::{self, LayaCheckpoint, LayaError};
use crate::state::log_message;
use crate::ui::{Modal, BTN, BTN_PRIMARY};

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Running(LayaCheckpoint),
    Done,
}

/// Which checkpoint the overlay is showing; `None` = hidden.
static OPEN_FOR: GlobalSignal<Option<LayaCheckpoint>> = Signal::global(|| None);
static PHASE: GlobalSignal<Phase> = Signal::global(|| Phase::Idle);
static PROGRESS: GlobalSignal<(u64, u64)> = Signal::global(|| (0, 0));
static ERROR: GlobalSignal<String> = Signal::global(String::new);
/// One download at a time (`start` refuses while one runs), so one flag.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// Show the download overlay for `checkpoint`. If a download is already
/// running, the overlay reattaches to it instead of resetting.
pub fn open(checkpoint: LayaCheckpoint) {
    if let Phase::Running(active) = *PHASE.read() {
        *OPEN_FOR.write() = Some(active);
        return;
    }
    *PHASE.write() = Phase::Idle;
    *PROGRESS.write() = (0, checkpoint.approx_total_bytes());
    *ERROR.write() = String::new();
    *OPEN_FOR.write() = Some(checkpoint);
}

/// Hide the overlay; an in-flight download is cancelled (the detached task
/// observes the flag, removes its staged files, and winds down).
pub fn close() {
    CANCEL.store(true, Ordering::Relaxed);
    *OPEN_FOR.write() = None;
}

/// True while the overlay is visible (the Escape hotkey closes it first).
pub fn is_open() -> bool {
    OPEN_FOR.read().is_some()
}

fn start() {
    if matches!(*PHASE.read(), Phase::Running(_)) {
        return;
    }
    let Some(cp) = *OPEN_FOR.read() else { return };
    CANCEL.store(false, Ordering::Relaxed);
    *ERROR.write() = String::new();
    *PROGRESS.write() = (0, cp.approx_total_bytes());
    *PHASE.write() = Phase::Running(cp);
    // Detached from any component scope: closing the overlay must not drop
    // the future mid-await, or the staged files would never be cleaned up.
    dioxus::core::spawn_forever(async move {
        let result = laya_local::download(
            cp,
            |done, total| *PROGRESS.write() = (done, total),
            &CANCEL,
        )
        .await;
        match result {
            Ok(()) => {
                *PHASE.write() = Phase::Done;
                log_message(format!("Laya model `{}` installed.", cp.id()));
            }
            Err(LayaError::Cancelled) => {
                *PHASE.write() = Phase::Idle;
                *ERROR.write() = "Download cancelled.".to_string();
            }
            Err(e) => {
                *PHASE.write() = Phase::Idle;
                *ERROR.write() = e.to_string();
            }
        }
    });
}

/// Rendered unconditionally by the dialog host, AFTER the `DIALOG` slot, so
/// the overlay stacks on top of the dialog that opened it.
#[component]
pub fn LayaDownloadHost() -> Element {
    let Some(checkpoint) = *OPEN_FOR.read() else { return rsx! {} };

    let already = checkpoint.is_installed();
    let (done_bytes, total_bytes) = *PROGRESS.read();
    let pct = if total_bytes > 0 {
        (done_bytes as f64 / total_bytes as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    let mb = |b: u64| b as f64 / 1_000_000.0;
    let current = *PHASE.read();
    let running = matches!(current, Phase::Running(_));

    rsx! {
        Modal {
            title: "Download Laya Model",
            width: "max-w-md",
            draggable: false,
            on_close: move |_| close(),
            footer: rsx! {
                match current {
                    Phase::Idle => rsx! {
                        button { class: BTN, onclick: move |_| close(), "Close" }
                        button {
                            class: BTN_PRIMARY,
                            onclick: move |_| start(),
                            if already { "Re-download" } else { "Download" }
                        }
                    },
                    Phase::Running(_) => rsx! {
                        button {
                            class: BTN,
                            onclick: move |_| CANCEL.store(true, Ordering::Relaxed),
                            "Cancel"
                        }
                    },
                    Phase::Done => rsx! {
                        button { class: BTN_PRIMARY, onclick: move |_| close(), "Close" }
                    },
                }
            },
            div { class: "space-y-2 text-sm",
                p {
                    "Checkpoint: "
                    span { class: "font-medium", "{checkpoint.label()}" }
                }
                p { class: "text-xs text-muted-foreground",
                    "Downloaded once from huggingface.co (convaiinnovations/laya) into \
                     ~/.config/balina/models/laya"
                }
                if running {
                    div { class: "h-2 w-full overflow-hidden rounded bg-muted",
                        div {
                            class: "h-full bg-primary",
                            style: "width: {pct:.1}%;",
                        }
                    }
                    div { class: "text-xs text-muted-foreground",
                        "{mb(done_bytes):.0} MB of ~{mb(total_bytes):.0} MB ({pct:.0}%)"
                    }
                }
                if current == Phase::Done {
                    div { class: "text-xs text-green-700",
                        "Installed. The first inference loads the model (a few seconds)."
                    }
                }
                if !ERROR.read().is_empty() {
                    div { class: "text-xs whitespace-pre-wrap text-red-600", "{ERROR}" }
                }
            }
        }
    }
}
