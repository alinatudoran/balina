//! Learn CPTs from a case file (counting or EM). Synchronous — fast at
//! editor scale.

use bn_session::ops::learn::{LearnCptsOpts, LearnMethod};
use dioxus::prelude::*;

use crate::platform::{pick_case_file, CaseFile};
use crate::state::{close_dialog, exec_res, log_message, LAST_CASE_FILE};
use crate::ui::{Modal, NumberInput, BTN, BTN_PRIMARY, BTN_SM};

pub fn file_label(f: &Option<CaseFile>) -> String {
    f.as_ref().map(|f| f.label()).unwrap_or_else(|| "no file selected".into())
}

#[component]
pub fn LearnCptsDialog() -> Element {
    let mut file: Signal<Option<CaseFile>> = use_signal(|| LAST_CASE_FILE.peek().clone());
    let mut method = use_signal(|| LearnMethod::Counting);
    let mut em_iters = use_signal(|| 50usize);
    let mut create_nodes = use_signal(|| true);
    let mut bins = use_signal(|| 4usize);
    let mut report: Signal<Option<String>> = use_signal(|| None);

    let run = move |_| {
        let Some(f) = file.read().clone() else { return };
        let opts = LearnCptsOpts {
            method: *method.read(),
            em_iters: *em_iters.read(),
            create_nodes: *create_nodes.read(),
            bins: *bins.read(),
        };
        spawn(async move {
            let bytes = match f.read().await {
                Ok(b) => b,
                Err(e) => {
                    report.set(Some(format!("Learning failed: {e}")));
                    return;
                }
            };
            let delim = f.forced_delim();
            match exec_res(|s| bn_session::ops::learn::learn_cpts(s, &bytes, delim, opts)) {
                Ok(res) => {
                    log_message(res.summary);
                    report.set(Some(res.report));
                    *crate::canvas::controller::FIT_REQUEST.write() += 1;
                }
                Err(e) => report.set(Some(format!("Learning failed: {e}"))),
            }
        });
    };

    rsx! {
        Modal {
            title: "Learn CPTs from cases",
            draggable: false, // configuration dialog
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Close" }
                button {
                    class: BTN_PRIMARY,
                    disabled: file.read().is_none(),
                    onclick: run,
                    "Learn"
                }
            },
            div { class: "space-y-3 text-sm",
                div { class: "flex items-center gap-2",
                    button {
                        class: BTN_SM,
                        onclick: move |_| {
                            spawn(async move {
                                if let Some(f) = pick_case_file().await {
                                    file.set(Some(f));
                                }
                            });
                        },
                        "Choose CSV/TSV…"
                    }
                    span { class: "truncate font-mono text-xs text-muted-foreground",
                        "{file_label(&file.read())}"
                    }
                }
                div { class: "flex items-center gap-1.5",
                    input {
                        id: "lc-create",
                        r#type: "checkbox",
                        checked: *create_nodes.read(),
                        onchange: move |ev| create_nodes.set(ev.checked()),
                    }
                    label { r#for: "lc-create", "Add missing nodes from columns" }
                }
                if *create_nodes.read() {
                    div { class: "flex items-center gap-2",
                        label { "Bins for continuous variables:" }
                        NumberInput {
                            min: "2".to_string(),
                            max: "20".to_string(),
                            value: bins.read().to_string(),
                            onchange: move |v: String| {
                                if let Ok(v) = v.parse::<usize>() {
                                    bins.set(v.clamp(2, 20));
                                }
                            },
                        }
                    }
                }
                div { class: "flex gap-4",
                    for (m, mid, label) in [
                        (LearnMethod::Counting, "lc-counting", "Counting"),
                        (LearnMethod::Em, "lc-em", "EM (missing data)"),
                    ] {
                        div { class: "flex items-center gap-1.5",
                            input {
                                id: mid,
                                r#type: "radio",
                                name: "lc-method",
                                checked: *method.read() == m,
                                onchange: move |_| method.set(m),
                            }
                            label { r#for: mid, "{label}" }
                        }
                    }
                }
                if *method.read() == LearnMethod::Em {
                    div { class: "flex items-center gap-2",
                        label { "Max iterations:" }
                        NumberInput {
                            class: "h-7 w-20".to_string(),
                            min: "1".to_string(),
                            max: "500".to_string(),
                            value: em_iters.read().to_string(),
                            onchange: move |v: String| {
                                if let Ok(v) = v.parse::<usize>() {
                                    em_iters.set(v.clamp(1, 500));
                                }
                            },
                        }
                    }
                }
                if let Some(r) = report.read().clone() {
                    pre { class: "max-h-40 overflow-auto rounded border bg-muted/30 p-2 \
                                  font-mono text-[11px] whitespace-pre-wrap select-text",
                        "{r}"
                    }
                }
            }
        }
    }
}
