//! CPT / utility table editor. Modeless (evidence clicks on the canvas stay
//! live). Draft-based: Cancel discards, Apply/OK writes one undoable step.
//! Cells keep a per-focus string buffer so "0." survives mid-typing.

use bn_core::model::NodeId;
use dioxus::prelude::*;

use crate::logic::cpt_math::{
    clamp_cell, format_cell, normalize_rows, row_sum, row_sum_ok, uniform,
};
use crate::state::{close_dialog, exec_res, SESSION, TYPING};
use crate::ui::{Modeless, BTN_SM, BTN_PRIMARY};

/// "Suggest with AI" row — desktop only; the web build renders nothing.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn ai_suggest_row(
    node_name: String,
    node_title: String,
    node_comment: String,
    states: Vec<String>,
    parent_names: Vec<String>,
    row_labels: Vec<String>,
    draft: Signal<Vec<f64>>,
    ai_cells: Signal<Vec<bool>>,
) -> Element {
    rsx! {
        AiSuggestRow {
            node_name,
            node_title,
            node_comment,
            states,
            parent_names,
            row_labels,
            draft,
            ai_cells,
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
fn ai_suggest_row(
    _node_name: String,
    _node_title: String,
    _node_comment: String,
    _states: Vec<String>,
    _parent_names: Vec<String>,
    _row_labels: Vec<String>,
    _draft: Signal<Vec<f64>>,
    _ai_cells: Signal<Vec<bool>>,
) -> Element {
    rsx! {}
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
fn AiSuggestRow(
    node_name: String,
    node_title: String,
    node_comment: String,
    states: Vec<String>,
    parent_names: Vec<String>,
    row_labels: Vec<String>,
    draft: Signal<Vec<f64>>,
    ai_cells: Signal<Vec<bool>>,
) -> Element {
    let mut guidance = use_signal(String::new);
    let mut running = use_signal(|| false);
    let mut status = use_signal(String::new);
    let mut failed = use_signal(|| false);

    let on_suggest = move |_| {
        let prompt = crate::ai::cpt::CptPrompt {
            node_name: node_name.clone(),
            node_title: node_title.clone(),
            node_comment: node_comment.clone(),
            states: states.clone(),
            parent_names: parent_names.clone(),
            row_labels: row_labels.clone(),
            guidance: guidance.read().clone(),
        };
        let cfg = crate::ai::settings::load().structured;
        let mut draft = draft;
        let mut ai_cells = ai_cells;
        running.set(true);
        failed.set(false);
        status.set("Asking the local structured model…".to_string());
        spawn(async move {
            let result = crate::ai::cpt::suggest_cpt(cfg, prompt).await;
            running.set(false);
            match result {
                Ok(data) => {
                    ai_cells.set(vec![true; data.len()]);
                    draft.set(data);
                    status.set(String::new());
                }
                Err(crate::ai::laya_local::LayaError::NotInstalled(checkpoint)) => {
                    // The download overlay stacks above this editor; the
                    // draft cells stay intact underneath.
                    crate::dialogs::laya_download::open(checkpoint);
                    status.set(
                        "The Laya model is not installed — download it, then press \
                         Suggest with AI again."
                            .to_string(),
                    );
                }
                Err(e) => {
                    failed.set(true);
                    status.set(e.to_string());
                }
            }
        });
    };

    rsx! {
        div { class: "flex items-center gap-2",
            crate::ui::TextInput {
                class: "h-7 flex-1 text-xs",
                value: guidance.read().clone(),
                placeholder: "Optional guidance, e.g. \u{201c}cancer is very likely if both parents are yes\u{201d}",
                oninput: move |v| guidance.set(v),
            }
            button {
                class: BTN_SM,
                disabled: *running.read(),
                onclick: on_suggest,
                if *running.read() { "Suggesting…" } else { "Suggest with AI" }
            }
        }
        if !status.read().is_empty() {
            div {
                class: if *failed.read() {
                    "max-h-24 overflow-auto text-xs whitespace-pre-wrap text-red-600"
                } else {
                    "text-xs text-muted-foreground"
                },
                "{status}"
            }
        }
        if ai_cells.read().iter().any(|&b| b) {
            div { class: "rounded border border-amber-300 bg-amber-50 px-2 py-1 text-xs text-amber-800",
                "AI-suggested values — review and edit freely (edited cells count as yours). \
                 Apply/OK writes a weak prior (experience \u{03b5} = 1 per row) on nodes without \
                 learned experience, so learned data overrides the suggestion."
            }
        }
    }
}

#[component]
pub fn CptEditorDialog(id: NodeId) -> Element {
    // Snapshot the labels/headers at open; the draft is the working copy.
    let Some((view, node_title, node_comment)) = use_hook(|| {
        crate::state::exec(|s| {
            let view = bn_session::ops::cpt::get_cpt(s, id)?;
            let n = s.doc().net.node(id);
            Ok((std::rc::Rc::new(view), n.title.clone(), n.comment.clone()))
        })
    }) else {
        close_dialog();
        return rsx! {};
    };
    let mut draft = use_signal(|| {
        view.rows.iter().flat_map(|r| r.values.iter().copied()).collect::<Vec<f64>>()
    });
    // The one focused cell's raw text (flat index, buffer).
    let mut editing: Signal<Option<(usize, String)>> = use_signal(|| None);
    // Per-cell provenance: true while a cell still holds the structured
    // model's value. Committing a cell by hand clears its flag; as long as
    // ANY remains, accepting writes through ops::cpt::set_cpt_ai (weak
    // Dirichlet prior). A fully hand-edited grid goes through set_cpt.
    let mut ai_cells: Signal<Vec<bool>> = use_signal(Vec::new);
    // Error from a failed Apply/OK, shown inline (the message log is easy to
    // miss and OK must not close over a failed write).
    let mut apply_error = use_signal(String::new);

    let out = view.out_card;
    let is_utility = view.is_utility;
    let title = format!("{}: {}", if is_utility { "Utility" } else { "CPT" }, view.node_name);
    let n_rows = view.rows.len();

    // Row labels for the elicitation prompt: "Smoking=yes, Bronchitis=no".
    let row_labels: Vec<String> = view
        .rows
        .iter()
        .map(|r| {
            if r.labels.is_empty() {
                "(no parents)".to_string()
            } else {
                view.parent_headers
                    .iter()
                    .zip(r.labels.iter())
                    .map(|(p, l)| format!("{p}={l}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        })
        .collect();

    let mut commit_cell = move |idx: usize, text: String| {
        if let Ok(v) = text.trim().parse::<f64>()
            && v.is_finite() {
                draft.write()[idx] = clamp_cell(v, is_utility);
                // A hand-committed cell is the user's value now.
                if let Some(flag) = ai_cells.write().get_mut(idx) {
                    *flag = false;
                }
            }
        editing.set(None);
    };

    let mut do_apply = move || -> bool {
        let data = draft.read().clone();
        // The editor is modeless: the node's structure can change while it
        // is open. A stale draft must fail loudly, not land half-applied.
        let expected = {
            let s = SESSION.read();
            if !s.doc().net.contains(id) {
                return false; // the host's close-on-vanish is about to fire
            }
            s.doc().net.table_len(id)
        };
        if data.len() != expected {
            apply_error.set(
                "The node's parents or states changed while this editor was open — \
                 the draft no longer fits. Close and reopen the editor."
                    .to_string(),
            );
            return false;
        }
        let result = if ai_cells.read().iter().any(|&b| b) {
            exec_res(|s| bn_session::ops::cpt::set_cpt_ai(s, id, data))
        } else {
            exec_res(|s| bn_session::ops::cpt::set_cpt(s, id, data))
        };
        match result {
            Ok(()) => {
                apply_error.set(String::new());
                true
            }
            Err(e) => {
                apply_error.set(e.to_string());
                false
            }
        }
    };
    let apply = move |_| {
        do_apply();
    };
    let apply_close = move |_| {
        // OK keeps the editor open when the write failed — closing would
        // leave the user believing the table was written.
        if do_apply() {
            close_dialog();
        }
    };

    rsx! {
        Modeless {
            title,
            width: "max-w-3xl",
            on_close: move |_| close_dialog(),
            div { class: "flex items-center gap-2",
                if !is_utility {
                    button {
                        class: BTN_SM,
                        onclick: move |_| {
                            let next = normalize_rows(&draft.read(), out);
                            draft.set(next);
                        },
                        "Normalize rows"
                    }
                    button {
                        class: BTN_SM,
                        onclick: move |_| {
                            let len = draft.read().len();
                            draft.set(uniform(len, out));
                            // Nothing AI-suggested survives a uniform reset.
                            ai_cells.set(Vec::new());
                        },
                        "Uniform"
                    }
                }
                div { class: "ml-auto flex gap-2",
                    button { class: BTN_SM, onclick: move |_| close_dialog(), "Cancel" }
                    button { class: BTN_SM, onclick: apply, "Apply" }
                    button { class: "{BTN_PRIMARY} h-7 text-xs", onclick: apply_close, "OK" }
                }
            }
            if !is_utility {
                {ai_suggest_row(
                    view.node_name.clone(),
                    node_title.clone(),
                    node_comment.clone(),
                    view.column_headers.clone(),
                    view.parent_headers.clone(),
                    row_labels.clone(),
                    draft,
                    ai_cells,
                )}
            }
            if !apply_error.read().is_empty() {
                div { class: "max-h-24 overflow-auto text-xs whitespace-pre-wrap text-red-600",
                    "{apply_error}"
                }
            }
            div { class: "max-h-[60vh] overflow-auto rounded border",
                table { class: "w-full text-xs",
                    thead { class: "sticky top-0 bg-muted",
                        tr {
                            for p in view.parent_headers.iter() {
                                th { class: "px-2 py-1.5 text-left font-semibold", "{p}" }
                            }
                            for c in view.column_headers.iter() {
                                th { class: "px-2 py-1.5 text-left font-semibold text-amber-700",
                                    "{c}"
                                }
                            }
                            if !is_utility {
                                th { class: "px-2 py-1.5 text-left font-semibold", "Σ" }
                            }
                        }
                    }
                    tbody {
                        for r in 0..n_rows {
                            tr { class: "border-t odd:bg-muted/30",
                                for l in view.rows[r].labels.iter() {
                                    td { class: "px-2 py-0.5 whitespace-nowrap", "{l}" }
                                }
                                for c in 0..out {
                                    {
                                        let idx = r * out + c;
                                        let cell_text = match &*editing.read() {
                                            Some((ei, text)) if *ei == idx => text.clone(),
                                            _ => format_cell(draft.read()[idx]),
                                        };
                                        rsx! {
                                            td { class: "px-1 py-0.5",
                                                input {
                                                    class: "h-6 w-20 rounded border bg-background px-1 \
                                                            text-right font-mono text-xs outline-none \
                                                            focus:ring-2 focus:ring-ring/50",
                                                    r#type: "text",
                                                    inputmode: "decimal",
                                                    value: "{cell_text}",
                                                    onfocus: move |_| {
                                                        *TYPING.write() = true;
                                                        let cur = draft.read()[idx].to_string();
                                                        editing.set(Some((idx, cur)));
                                                    },
                                                    oninput: move |ev| {
                                                        editing.set(Some((idx, ev.value())));
                                                    },
                                                    onblur: move |_| {
                                                        *TYPING.write() = false;
                                                        // Bind first — commit_cell writes `editing`,
                                                        // which must not still be read-borrowed.
                                                        let cur = editing.read().clone();
                                                        if let Some((ei, text)) = cur
                                                            && ei == idx {
                                                                commit_cell(idx, text);
                                                            }
                                                    },
                                                    onkeydown: move |ev| {
                                                        if ev.data().key()
                                                            != dioxus::html::input_data::keyboard_types::Key::Enter
                                                        {
                                                            return;
                                                        }
                                                        let cur = editing.read().clone();
                                                        if let Some((ei, text)) = cur
                                                            && ei == idx {
                                                                commit_cell(idx, text);
                                                            }
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                                if !is_utility {
                                    {
                                        let sum = row_sum(&draft.read(), r, out);
                                        let cls = if row_sum_ok(sum) {
                                            "px-2 py-0.5 font-mono text-emerald-600"
                                        } else {
                                            "px-2 py-0.5 font-mono text-red-600"
                                        };
                                        rsx! {
                                            td { class: cls, "{sum:.3}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
