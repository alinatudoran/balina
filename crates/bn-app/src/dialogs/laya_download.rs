//! Download dialog for a local Laya checkpoint: explicit start, streamed
//! progress, cancel (partial files cleaned up). Opened from AI Settings and
//! from any structured feature that finds the model missing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dioxus::prelude::*;

use crate::ai::laya_local::{self, LayaCheckpoint, LayaError};
use crate::state::{close_dialog, log_message};
use crate::ui::{BTN, BTN_PRIMARY, Modal};

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Running,
    Done,
}

#[component]
pub fn LayaDownloadDialog(checkpoint: LayaCheckpoint) -> Element {
    let mut phase = use_signal(|| Phase::Idle);
    let progress = use_signal(|| (0u64, checkpoint.approx_total_bytes()));
    let mut error = use_signal(String::new);
    let cancel = use_hook(|| Arc::new(AtomicBool::new(false)));

    let already = checkpoint.is_installed();

    let start = {
        let cancel = cancel.clone();
        move |_| {
            let cancel = cancel.clone();
            cancel.store(false, Ordering::Relaxed);
            phase.set(Phase::Running);
            error.set(String::new());
            spawn(async move {
                let result = laya_local::download(
                    checkpoint,
                    move |done, total| {
                        // Signals are Copy; rebind mutably inside the Fn.
                        let mut progress = progress;
                        progress.set((done, total));
                    },
                    cancel,
                )
                .await;
                match result {
                    Ok(()) => {
                        phase.set(Phase::Done);
                        log_message(format!("Laya model `{}` installed.", checkpoint.id()));
                    }
                    Err(LayaError::Cancelled) => {
                        phase.set(Phase::Idle);
                        error.set("Download cancelled.".into());
                    }
                    Err(e) => {
                        phase.set(Phase::Idle);
                        error.set(e.to_string());
                    }
                }
            });
        }
    };

    // Closing the dialog cancels an in-flight download.
    let on_close = {
        let cancel = cancel.clone();
        move |_| {
            cancel.store(true, Ordering::Relaxed);
            close_dialog();
        }
    };
    let cancel_click = {
        let cancel = cancel.clone();
        move |_| cancel.store(true, Ordering::Relaxed)
    };

    let (done_bytes, total_bytes) = *progress.read();
    let pct = if total_bytes > 0 {
        (done_bytes as f64 / total_bytes as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    let mb = |b: u64| b as f64 / 1_000_000.0;
    let current = *phase.read();

    rsx! {
        Modal {
            title: "Download Laya Model",
            width: "max-w-md",
            draggable: false,
            on_close,
            footer: rsx! {
                match current {
                    Phase::Idle => rsx! {
                        button { class: BTN, onclick: move |_| close_dialog(), "Close" }
                        button {
                            class: BTN_PRIMARY,
                            onclick: start,
                            if already { "Re-download" } else { "Download" }
                        }
                    },
                    Phase::Running => rsx! {
                        button { class: BTN, onclick: cancel_click, "Cancel" }
                    },
                    Phase::Done => rsx! {
                        button { class: BTN_PRIMARY, onclick: move |_| close_dialog(), "Close" }
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
                if current == Phase::Running {
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
                if !error.read().is_empty() {
                    div { class: "text-xs whitespace-pre-wrap text-red-600", "{error}" }
                }
            }
        }
    }
}
