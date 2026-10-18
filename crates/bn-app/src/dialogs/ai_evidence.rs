//! Enter-findings-from-text dialog: paste a case note, the structured model
//! judges every chance node (with an explicit abstain option), and the
//! suggested distributions land in a review list — editable, per-node
//! include checkboxes — applied as likelihood findings in one undoable step.

use dioxus::prelude::*;

use crate::ai::{evidence, settings};
use crate::state::{close_dialog, exec, log_message, SESSION};
use crate::ui::{Modal, TextArea, TextInput, BTN, BTN_PRIMARY};

/// One reviewed suggestion; values are kept as strings so the user can edit
/// them like the likelihood dialog's cells.
#[derive(Clone, PartialEq)]
struct Review {
    name: String,
    title: String,
    states: Vec<String>,
    values: Vec<String>,
    include: bool,
}

#[component]
pub fn AiEvidenceDialog() -> Element {
    let mut text = use_signal(String::new);
    let mut running = use_signal(|| false);
    let mut error = use_signal(String::new);
    // None = paste step; Some = review step (with the abstained node names).
    let mut review: Signal<Option<(Vec<Review>, Vec<String>)>> = use_signal(|| None);

    let on_analyze = move |_| {
        let nodes: Vec<evidence::NodeQuery> = {
            let s = SESSION.read();
            let net = &s.doc().net;
            net.node_ids()
                .into_iter()
                .filter(|&id| net.node(id).kind == bn_core::model::NodeKind::Chance)
                .map(|id| {
                    let n = net.node(id);
                    evidence::NodeQuery {
                        name: n.name.clone(),
                        title: n.title.clone(),
                        comment: n.comment.clone(),
                        states: n.states.iter().map(|st| st.name.clone()).collect(),
                    }
                })
                .collect()
        };
        let doc_text = text.read().trim().to_string();
        let cfg = settings::load().structured;
        running.set(true);
        error.set(String::new());
        spawn(async move {
            let result = evidence::suggest_evidence(cfg, nodes.clone(), doc_text).await;
            running.set(false);
            match result {
                Err(crate::ai::laya_local::LayaError::NotInstalled(checkpoint)) => {
                    // Hand over to the download dialog (single dialog slot);
                    // the user reopens this dialog after installing.
                    crate::state::open_dialog(crate::state::DialogDesc::LayaDownload {
                        checkpoint,
                    });
                }
                Ok(found) => {
                    let found_names: Vec<&String> = found.iter().map(|(n, _)| n).collect();
                    let abstained: Vec<String> = nodes
                        .iter()
                        .map(|n| n.name.clone())
                        .filter(|n| !found_names.contains(&n))
                        .collect();
                    let reviews: Vec<Review> = found
                        .iter()
                        .map(|(name, probs)| {
                            let q = nodes.iter().find(|n| n.name == *name).expect("validated");
                            Review {
                                name: name.clone(),
                                title: q.title.clone(),
                                states: q.states.clone(),
                                values: probs.iter().map(|p| format!("{p:.3}")).collect(),
                                include: true,
                            }
                        })
                        .collect();
                    if reviews.is_empty() {
                        error.set("The model found no information about any node in this text."
                            .to_string());
                    } else {
                        review.set(Some((reviews, abstained)));
                    }
                }
                Err(e) => error.set(e.to_string()),
            }
        });
    };

    let apply = move |_| {
        let Some((reviews, _)) = review.read().clone() else { return };
        let findings: Vec<(bn_core::model::NodeId, Vec<f64>)> = {
            let s = SESSION.read();
            reviews
                .iter()
                .filter(|r| r.include)
                .filter_map(|r| {
                    let id = s.doc().net.find_by_name(&r.name)?;
                    let vals: Vec<f64> =
                        r.values.iter().filter_map(|v| v.trim().parse::<f64>().ok()).collect();
                    (vals.len() == r.states.len()).then_some((id, vals))
                })
                .collect()
        };
        let n = findings.len();
        if exec(|s| bn_session::ops::evidence::set_likelihood_findings(s, findings)).is_some() {
            log_message(format!("Applied {n} AI-extracted likelihood findings."));
            close_dialog();
        }
    };

    let reviewing = review.read().is_some();
    let can_analyze = !*running.read() && !text.read().trim().is_empty();
    // Included rows must parse to valid likelihoods (≥ 0, one > 0).
    let can_apply = review.read().as_ref().is_some_and(|(rs, _)| {
        let included: Vec<&Review> = rs.iter().filter(|r| r.include).collect();
        !included.is_empty()
            && included.iter().all(|r| {
                let parsed: Vec<Option<f64>> =
                    r.values.iter().map(|v| v.trim().parse::<f64>().ok()).collect();
                parsed.iter().all(|v| v.is_some_and(|v| v.is_finite() && v >= 0.0))
                    && parsed.iter().any(|v| v.is_some_and(|v| v > 0.0))
            })
    });

    rsx! {
        Modal {
            title: "Enter Findings from Text",
            width: "max-w-2xl",
            draggable: false,
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                if reviewing {
                    button {
                        class: BTN,
                        onclick: move |_| {
                            review.set(None);
                            error.set(String::new());
                        },
                        "Back"
                    }
                    button { class: BTN_PRIMARY, disabled: !can_apply, onclick: apply, "Apply" }
                } else {
                    button {
                        class: BTN_PRIMARY,
                        disabled: !can_analyze,
                        onclick: on_analyze,
                        if *running.read() { "Analyzing…" } else { "Analyze" }
                    }
                }
            },
            div { class: "space-y-2 text-sm",
                if !reviewing {
                    p { class: "text-xs text-muted-foreground",
                        "Paste a case note or document. The local Laya model judges what the \
                         text says about each chance node (or abstains); you review and edit \
                         the distributions before they become likelihood findings. Nothing \
                         leaves this machine."
                    }
                    TextArea {
                        rows: 10,
                        value: text.read().clone(),
                        autofocus: true,
                        oninput: move |v| text.set(v),
                    }
                    if *running.read() {
                        div { class: "text-xs text-muted-foreground",
                            "Asking the local structured model (first run loads it — a few \
                             seconds)…"
                        }
                    }
                } else if let Some((reviews, abstained)) = review.read().clone() {
                    p { class: "text-xs text-muted-foreground",
                        "AI-suggested likelihoods — review, edit, untick what you don't trust. \
                         Apply sets them as soft evidence (one undo step)."
                    }
                    div { class: "max-h-[50vh] space-y-2 overflow-auto pr-1",
                        for (ri, r) in reviews.iter().cloned().enumerate() {
                            div { class: "rounded border p-2",
                                label { class: "flex items-center gap-2 font-medium",
                                    input {
                                        r#type: "checkbox",
                                        checked: r.include,
                                        onchange: move |ev| {
                                            if let Some((rs, _)) = review.write().as_mut() {
                                                rs[ri].include = ev.checked();
                                            }
                                        },
                                    }
                                    span { "{r.name}" }
                                    if !r.title.is_empty() {
                                        span { class: "text-xs font-normal text-muted-foreground",
                                            "{r.title}"
                                        }
                                    }
                                }
                                div { class: "mt-1 flex flex-wrap gap-x-4 gap-y-1 pl-6",
                                    for (si, sname) in r.states.iter().cloned().enumerate() {
                                        div { class: "flex items-center gap-1",
                                            span { class: "text-xs text-muted-foreground",
                                                "{sname}"
                                            }
                                            TextInput {
                                                class: "h-6 w-16 text-right font-mono text-xs",
                                                value: r.values[si].clone(),
                                                oninput: move |v| {
                                                    if let Some((rs, _)) = review.write().as_mut() {
                                                        rs[ri].values[si] = v;
                                                    }
                                                },
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !abstained.is_empty() {
                        p { class: "text-xs text-muted-foreground",
                            "No information found for: {abstained.join(\", \")}"
                        }
                    }
                }
                if !error.read().is_empty() {
                    div { class: "max-h-32 overflow-auto text-xs whitespace-pre-wrap text-red-600",
                        "{error}"
                    }
                }
            }
        }
    }
}
