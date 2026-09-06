//! CPT / utility table editor. Modeless (evidence clicks on the canvas stay
//! live). Draft-based: Cancel discards, Apply/OK writes one undoable step.
//! Cells keep a per-focus string buffer so "0." survives mid-typing.

use bn_core::model::NodeId;
use dioxus::prelude::*;

use crate::logic::cpt_math::{
    clamp_cell, format_cell, normalize_rows, row_sum, row_sum_ok, uniform,
};
use crate::state::{close_dialog, exec, TYPING};
use crate::ui::{Modeless, BTN_SM, BTN_PRIMARY};

#[component]
pub fn CptEditorDialog(id: NodeId) -> Element {
    // Snapshot the labels/headers at open; the draft is the working copy.
    let Some(view) = use_hook(|| {
        crate::state::exec(|s| bn_session::ops::cpt::get_cpt(s, id)).map(std::rc::Rc::new)
    }) else {
        close_dialog();
        return rsx! {};
    };
    let mut draft = use_signal(|| {
        view.rows.iter().flat_map(|r| r.values.iter().copied()).collect::<Vec<f64>>()
    });
    // The one focused cell's raw text (flat index, buffer).
    let mut editing: Signal<Option<(usize, String)>> = use_signal(|| None);

    let out = view.out_card;
    let is_utility = view.is_utility;
    let title = format!("{}: {}", if is_utility { "Utility" } else { "CPT" }, view.node_name);
    let n_rows = view.rows.len();

    let mut commit_cell = move |idx: usize, text: String| {
        if let Ok(v) = text.trim().parse::<f64>()
            && v.is_finite() {
                draft.write()[idx] = clamp_cell(v, is_utility);
            }
        editing.set(None);
    };

    let apply = move |_| {
        let data = draft.read().clone();
        exec(|s| bn_session::ops::cpt::set_cpt(s, id, data));
    };
    let apply_close = move |_| {
        let data = draft.read().clone();
        exec(|s| bn_session::ops::cpt::set_cpt(s, id, data));
        close_dialog();
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
