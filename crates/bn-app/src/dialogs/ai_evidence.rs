//! Enter-findings-from-text dialog: paste a case note, the structured model
//! judges every chance node (with an explicit abstain option), and the
//! suggested distributions land in a review list — editable, per-node
//! include checkboxes — applied in one undoable step. When a text model is
//! configured, an optional second pass marks findings the text states
//! outright; those snap to 1/0 and are applied as hard findings.

use std::collections::HashMap;

use bn_core::inference::Finding;
use bn_core::model::NodeId;
use bn_session::TabId;
use dioxus::prelude::*;

use crate::ai::{client, definite, evidence, settings};
use crate::state::{close_dialog, exec_res, log_message, SESSION};
use crate::ui::{Modal, TextArea, TextInput, BTN, BTN_PRIMARY};

/// One reviewed suggestion; values are kept as strings so the user can edit
/// them like the likelihood dialog's cells.
#[derive(Clone, PartialEq)]
struct Review {
    /// Captured at analyze time — Apply addresses the node by id, never by a
    /// name lookup against whatever document happens to be active.
    id: NodeId,
    name: String,
    title: String,
    states: Vec<String>,
    values: Vec<String>,
    include: bool,
    /// Text model confirmed the state; applied as a hard finding. Editing
    /// any value clears it.
    definite: bool,
}

#[component]
pub fn AiEvidenceDialog() -> Element {
    let mut text = use_signal(String::new);
    let mut running = use_signal(|| false);
    let mut error = use_signal(String::new);
    // None = paste step; Some = review step: the tab the analysis ran
    // against (Apply refuses on any other tab — node ids are per-document
    // and hotkeys can switch tabs under this modal), the reviews, and the
    // abstained node names.
    type ReviewStep = Option<(TabId, Vec<Review>, Vec<String>)>;
    let mut review: Signal<ReviewStep> = use_signal(|| None);
    // Text model usable for the definite-findings pass (provider label if so).
    let text_provider = use_hook(|| {
        let t = settings::load().text;
        client::build_llm_model(&t).is_ok().then(|| t.provider.label())
    });
    let mut use_definite = use_signal(|| true);
    let mut progress = use_signal(String::new);
    let mut notice = use_signal(String::new);

    let on_analyze = move |_| {
        let (tab, ids, nodes): (TabId, HashMap<String, NodeId>, Vec<evidence::NodeQuery>) = {
            let s = SESSION.read();
            let net = &s.doc().net;
            let mut ids = HashMap::new();
            let nodes = net
                .node_ids()
                .into_iter()
                .filter(|&id| net.node(id).kind == bn_core::model::NodeKind::Chance)
                .map(|id| {
                    let n = net.node(id);
                    ids.insert(n.name.clone(), id);
                    evidence::NodeQuery {
                        name: n.name.clone(),
                        title: n.title.clone(),
                        comment: n.comment.clone(),
                        states: n.states.iter().map(|st| st.name.clone()).collect(),
                    }
                })
                .collect();
            (s.active_id(), ids, nodes)
        };
        let doc_text = text.read().trim().to_string();
        let all = settings::load();
        let (cfg, text_cfg) = (all.structured, all.text);
        let check_definite = text_provider.is_some() && *use_definite.read();
        running.set(true);
        error.set(String::new());
        notice.set(String::new());
        progress.set(
            "Asking the local structured model (first run loads it — a few seconds)…".into(),
        );
        spawn(async move {
            let result =
                evidence::suggest_evidence(cfg, nodes.clone(), doc_text.clone()).await;
            let mut definite_found: Vec<(String, usize)> = Vec::new();
            if check_definite
                && let Ok(found) = &result
                && !found.is_empty()
            {
                progress.set("Checking definite findings with the Text model…".into());
                let queried: Vec<(evidence::NodeQuery, Vec<f64>)> = found
                    .iter()
                    .filter_map(|(name, probs)| {
                        let q = nodes.iter().find(|n| n.name == *name)?;
                        Some((q.clone(), probs.clone()))
                    })
                    .collect();
                match definite::detect_definite(text_cfg, doc_text, queried).await {
                    Ok(d) => definite_found = d,
                    Err(e) => notice
                        .set(format!("Text model check failed ({e}); showing Laya values.")),
                }
            }
            running.set(false);
            match result {
                Err(crate::ai::laya_local::LayaError::NotInstalled(checkpoint)) => {
                    // The download overlay stacks above this dialog; the
                    // pasted text stays intact underneath.
                    crate::dialogs::laya_download::open(checkpoint);
                    error.set(
                        "The Laya model is not installed — download it, then press \
                         Analyze again."
                            .to_string(),
                    );
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
                            let hard = definite_found
                                .iter()
                                .find(|(n, _)| n == name)
                                .map(|&(_, i)| i);
                            let values = match hard {
                                Some(i) => (0..probs.len())
                                    .map(|k| if k == i { "1" } else { "0" }.to_string())
                                    .collect(),
                                None => probs.iter().map(|p| format!("{p:.3}")).collect(),
                            };
                            Review {
                                id: *ids.get(name).expect("validated"),
                                name: name.clone(),
                                title: q.title.clone(),
                                states: q.states.clone(),
                                values,
                                include: true,
                                definite: hard.is_some(),
                            }
                        })
                        .collect();
                    if reviews.is_empty() {
                        error.set("The model found no information about any node in this text."
                            .to_string());
                    } else {
                        review.set(Some((tab, reviews, abstained)));
                    }
                }
                Err(e) => error.set(e.to_string()),
            }
        });
    };

    let apply = move |_| {
        let Some((tab, reviews, _)) = review.read().clone() else { return };
        // Every failure is reported in the dialog, which stays open — an
        // Apply that silently does nothing (or applies to the wrong network)
        // is worse than an error.
        let (findings, missing) = {
            let s = SESSION.read();
            if s.active_id() != tab {
                error.set(
                    "A different network tab is active now. Switch back to the analyzed \
                     network (or re-run the analysis here) before applying."
                        .to_string(),
                );
                return;
            }
            let mut findings: Vec<(NodeId, Finding)> = Vec::new();
            let mut missing: Vec<String> = Vec::new();
            for r in reviews.iter().filter(|r| r.include) {
                if !s.doc().net.contains(r.id) {
                    missing.push(r.name.clone());
                    continue;
                }
                let vals: Vec<f64> =
                    r.values.iter().filter_map(|v| v.trim().parse::<f64>().ok()).collect();
                if vals.len() != r.states.len() {
                    // Unreachable while `can_apply` gates the button.
                    missing.push(r.name.clone());
                    continue;
                }
                // A definite row is one-hot by construction (edits clear the flag).
                let finding = match vals.iter().position(|&v| v > 0.0) {
                    Some(i) if r.definite => Finding::Hard(i),
                    _ => Finding::Likelihood(vals),
                };
                findings.push((r.id, finding));
            }
            (findings, missing)
        };
        if !missing.is_empty() {
            error.set(format!(
                "These nodes no longer exist in the network: {}. Untick them or re-run \
                 the analysis.",
                missing.join(", ")
            ));
            return;
        }
        let hard = findings.iter().filter(|(_, f)| matches!(f, Finding::Hard(_))).count();
        let soft = findings.len() - hard;
        match exec_res(|s| bn_session::ops::evidence::set_findings(s, findings)) {
            Ok(()) => {
                log_message(format!(
                    "Applied AI-extracted findings: {hard} hard, {soft} likelihood."
                ));
                close_dialog();
            }
            Err(e) => error.set(e.to_string()),
        }
    };

    let reviewing = review.read().is_some();
    let can_analyze = !*running.read() && !text.read().trim().is_empty();
    // Included rows must parse to valid likelihoods (≥ 0, one > 0).
    let can_apply = review.read().as_ref().is_some_and(|(_, rs, _)| {
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
                         the distributions before they become likelihood findings."
                        if text_provider.is_some() && *use_definite.read() {
                            " The text is also sent to the Text model to spot findings it states \
                             outright; those become hard findings."
                        } else {
                            " Nothing leaves this machine."
                        }
                    }
                    TextArea {
                        rows: 10,
                        value: text.read().clone(),
                        autofocus: true,
                        oninput: move |v| text.set(v),
                    }
                    if let Some(provider) = text_provider {
                        label { class: "flex items-center gap-2 text-xs",
                            input {
                                r#type: "checkbox",
                                checked: *use_definite.read(),
                                onchange: move |ev| use_definite.set(ev.checked()),
                            }
                            "Use the Text model to detect definite findings (sends the text to \
                             {provider})"
                        }
                    }
                    if *running.read() {
                        div { class: "text-xs text-muted-foreground", "{progress}" }
                    }
                } else if let Some((_, reviews, abstained)) = review.read().clone() {
                    p { class: "text-xs text-muted-foreground",
                        "AI-suggested likelihoods — review, edit, untick what you don't trust. \
                         Apply sets them as soft evidence, and rows marked definite as hard \
                         findings (one undo step)."
                    }
                    if !notice.read().is_empty() {
                        p { class: "text-xs text-amber-600", "{notice}" }
                    }
                    div { class: "max-h-[50vh] space-y-2 overflow-auto pr-1",
                        for (ri, r) in reviews.iter().cloned().enumerate() {
                            div { class: "rounded border p-2",
                                label { class: "flex items-center gap-2 font-medium",
                                    input {
                                        r#type: "checkbox",
                                        checked: r.include,
                                        onchange: move |ev| {
                                            if let Some((_, rs, _)) = review.write().as_mut() {
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
                                    if r.definite {
                                        span {
                                            class: "rounded bg-emerald-100 px-1 text-[10px] font-normal text-emerald-800",
                                            title: "The Text model found this stated outright — applied as a hard finding",
                                            "definite"
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
                                                    if let Some((_, rs, _)) = review.write().as_mut() {
                                                        rs[ri].values[si] = v;
                                                        rs[ri].definite = false;
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
