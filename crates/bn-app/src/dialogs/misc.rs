//! Small dialogs: likelihood finding, report/ID-solution text, network
//! rename, about.

use bn_core::model::NodeId;
use dioxus::prelude::*;

use crate::state::{close_dialog, exec, SESSION};
use crate::ui::{Modal, TextInput, BTN, BTN_PRIMARY};

/// Soft (likelihood) evidence: one ≥0 weight per state.
#[component]
pub fn LikelihoodDialog(id: NodeId) -> Element {
    let (name, state_names, initial) = {
        let s = SESSION.read();
        if !s.doc.net.contains(id) {
            return rsx! {};
        }
        let n = s.doc.net.node(id);
        let initial: Vec<String> = match s.doc.evidence.get(id) {
            Some(bn_core::inference::Finding::Likelihood(l)) => {
                n.states.iter().enumerate().map(|(i, _)| {
                    l.get(i).copied().unwrap_or(1.0).to_string()
                }).collect()
            }
            _ => n.states.iter().map(|_| "1".to_string()).collect(),
        };
        (
            n.name.clone(),
            n.states.iter().map(|st| st.name.clone()).collect::<Vec<_>>(),
            initial,
        )
    };
    let mut values = use_signal(|| initial);

    let parsed: Vec<Option<f64>> =
        values.read().iter().map(|v| v.trim().parse::<f64>().ok()).collect();
    let valid = parsed.iter().all(|v| v.is_some_and(|v| v.is_finite() && v >= 0.0))
        && parsed.iter().any(|v| v.is_some_and(|v| v > 0.0));

    rsx! {
        Modal {
            title: "Likelihood finding: {name}",
            width: "max-w-xs",
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button {
                    class: BTN_PRIMARY,
                    disabled: !valid,
                    onclick: move |_| {
                        let vals: Vec<f64> = values
                            .read()
                            .iter()
                            .filter_map(|v| v.trim().parse::<f64>().ok())
                            .collect();
                        exec(|s| bn_session::ops::evidence::set_likelihood_finding(s, id, vals));
                        close_dialog();
                    },
                    "Apply"
                }
            },
            div { class: "space-y-2 text-sm",
                for (i, sname) in state_names.iter().cloned().enumerate() {
                    div { class: "flex items-center gap-2",
                        span { class: "w-28 truncate", "{sname}" }
                        TextInput {
                            class: "h-7 w-24 text-xs",
                            value: values.read()[i].clone(),
                            oninput: move |v| values.write()[i] = v,
                        }
                    }
                }
                if !valid {
                    div { class: "text-xs text-red-600",
                        "values must be ≥ 0 with at least one > 0"
                    }
                }
            }
        }
    }
}

#[component]
pub fn ReportDialog(title: String, text: String) -> Element {
    rsx! {
        Modal { title, on_close: move |_| close_dialog(),
            pre { class: "max-h-80 overflow-auto rounded border bg-muted/30 p-2 font-mono \
                          text-[11px] whitespace-pre-wrap select-text",
                "{text}"
            }
        }
    }
}

#[component]
pub fn RenameNetworkDialog() -> Element {
    let mut name = use_signal(|| SESSION.read().doc.net.name.clone());
    let ok = move |_| {
        let n = name.read().clone();
        exec(|s| {
            bn_session::ops::edit::set_network_name(s, n);
            Ok(())
        });
        close_dialog();
    };
    rsx! {
        Modal {
            title: "Network name",
            width: "max-w-xs",
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button { class: BTN_PRIMARY, onclick: ok, "OK" }
            },
            TextInput {
                value: name.read().clone(),
                autofocus: true,
                oninput: move |v| name.set(v),
            }
        }
    }
}

#[component]
pub fn AboutDialog() -> Element {
    rsx! {
        Modal {
            title: "About Balina",
            width: "max-w-sm",
            on_close: move |_| close_dialog(),
            div { class: "space-y-1 text-sm",
                p { "Balina — a Bayesian network / influence diagram editor." }
                p { class: "text-muted-foreground text-xs",
                    "Version {env!(\"CARGO_PKG_VERSION\")}"
                }
            }
        }
    }
}
