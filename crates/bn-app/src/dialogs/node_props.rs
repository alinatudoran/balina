//! Node properties: name/title/comment/kind + the state list. State edits
//! build the `orig` remap array (index of the pre-edit state each draft row
//! came from) so the session can reshape this node's table and its children.

use bn_core::model::{ContinuousInfo, NodeId, NodeKind};
use bn_session::views::{NodePropsPatch, StateDraft};
use dioxus::prelude::*;

use crate::state::{close_dialog, exec_res, SESSION};
use crate::ui::{Modal, TextArea, TextInput, BTN, BTN_ICON, BTN_PRIMARY, BTN_SM, SELECT};

#[derive(Clone, PartialEq)]
struct StateRow {
    name: String,
    /// Raw text; parsed on apply.
    value: String,
    orig: Option<usize>,
}

#[component]
pub fn NodePropertiesDialog(id: NodeId) -> Element {
    let init = {
        let s = SESSION.read();
        if !s.doc.net.contains(id) {
            return rsx! {};
        }
        let n = s.doc.net.node(id);
        (
            n.name.clone(),
            n.title.clone(),
            n.comment.clone(),
            n.kind,
            n.states
                .iter()
                .enumerate()
                .map(|(i, st)| StateRow {
                    name: st.name.clone(),
                    value: st.value.map(|v| v.to_string()).unwrap_or_default(),
                    orig: Some(i),
                })
                .collect::<Vec<_>>(),
            n.states.len(),
            n.continuous.clone(),
        )
    };
    let (node_name, orig_state_count) = (init.0.clone(), init.5);
    let continuous_info: Option<ContinuousInfo> = init.6.clone();
    let mut name = use_signal(|| init.0.clone());
    let mut title = use_signal(|| init.1.clone());
    let mut comment = use_signal(|| init.2.clone());
    let mut kind = use_signal(|| init.3);
    let mut states = use_signal(|| init.4.clone());
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let child_count = {
        let s = SESSION.read();
        s.doc.net.nodes().filter(|(_, n)| n.parents.contains(&id)).count()
    };
    let remap_changed = states.read().len() != orig_state_count
        || states.read().iter().enumerate().any(|(i, r)| r.orig != Some(i));

    // Pre-format continuous stats for display (can't compute expressions inside rsx!).
    let continuous_display: Option<[String; 6]> = continuous_info.as_ref().map(|ci| {
        [
            format!("{:.4}", ci.mean),
            format!("{:.4}", ci.std),
            format!("{:.0}", ci.n),
            format!("{:.4}", ci.min),
            format!("{:.4}", ci.max),
            format!("{}", ci.edges.len() + 1),
        ]
    });

    let apply = move |_| {
        let patch = NodePropsPatch {
            name: name.read().clone(),
            title: title.read().clone(),
            comment: comment.read().clone(),
            kind: *kind.read(),
            states: states
                .read()
                .iter()
                .map(|r| StateDraft {
                    name: r.name.clone(),
                    value: r.value.trim().parse::<f64>().ok(),
                    orig: r.orig,
                })
                .collect(),
        };
        match exec_res(|s| bn_session::ops::edit::update_node_props(s, id, patch)) {
            Ok(()) => close_dialog(),
            Err(e) => error.set(Some(e.to_string())),
        }
    };

    let n_rows = states.read().len();

    rsx! {
        Modal {
            title: "Node: {node_name}",
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button { class: BTN_PRIMARY, onclick: apply, "OK" }
            },
            div { class: "grid grid-cols-[70px_1fr] items-center gap-2 text-sm",
                label { r#for: "np-name", "Name" }
                TextInput {
                    id: "np-name".to_string(),
                    value: name.read().clone(),
                    oninput: move |v| name.set(v),
                }
                label { r#for: "np-title", "Title" }
                TextInput {
                    id: "np-title".to_string(),
                    value: title.read().clone(),
                    oninput: move |v| title.set(v),
                }
                label { "Kind" }
                select {
                    class: SELECT,
                    onchange: move |ev| {
                        kind.set(match ev.value().as_str() {
                            "Decision" => NodeKind::Decision,
                            "Utility" => NodeKind::Utility,
                            _ => NodeKind::Chance,
                        });
                    },
                    option { value: "Chance", selected: *kind.read() == NodeKind::Chance,
                        "Chance (nature)"
                    }
                    option { value: "Decision", selected: *kind.read() == NodeKind::Decision,
                        "Decision"
                    }
                    option { value: "Utility", selected: *kind.read() == NodeKind::Utility,
                        "Utility"
                    }
                }
                label { class: "self-start pt-1.5", "Comment" }
                TextArea {
                    value: comment.read().clone(),
                    oninput: move |v| comment.set(v),
                }
            }
            if let Some(ref cd) = continuous_display {
                div { class: "col-span-2 space-y-1",
                    div { class: "inline-flex items-center gap-1.5 rounded-full bg-blue-100 px-2 py-0.5 text-xs font-medium text-blue-800",
                        "Continuous variable"
                    }
                    div { class: "grid grid-cols-3 gap-x-4 gap-y-0.5 rounded border bg-muted/30 p-2 font-mono text-[11px]",
                        span { "mean" }  span { "std" }  span { "n" }
                        span { "{cd[0]}" }  span { "{cd[1]}" }  span { "{cd[2]}" }
                        span { "min" }  span { "max" }  span { "bins" }
                        span { "{cd[3]}" }  span { "{cd[4]}" }  span { "{cd[5]}" }
                    }
                }
            }
            if *kind.read() != NodeKind::Utility {
                div { class: "space-y-1.5",
                    div { class: "text-xs text-muted-foreground",
                        "States (name / numeric value):"
                    }
                    for i in 0..n_rows {
                        div { class: "flex items-center gap-1",
                            TextInput {
                                class: "h-7 w-36 text-xs",
                                value: states.read()[i].name.clone(),
                                oninput: move |v| states.write()[i].name = v,
                            }
                            TextInput {
                                class: "h-7 w-20 text-xs",
                                placeholder: "value".to_string(),
                                value: states.read()[i].value.clone(),
                                oninput: move |v| states.write()[i].value = v,
                            }
                            button {
                                class: BTN_ICON,
                                disabled: i == 0,
                                onclick: move |_| states.write().swap(i - 1, i),
                                "↑"
                            }
                            button {
                                class: BTN_ICON,
                                disabled: i + 1 == n_rows,
                                onclick: move |_| states.write().swap(i, i + 1),
                                "↓"
                            }
                            button {
                                class: BTN_ICON,
                                disabled: n_rows <= 1,
                                onclick: move |_| {
                                    states.write().remove(i);
                                },
                                "✕"
                            }
                        }
                    }
                    button {
                        class: BTN_SM,
                        onclick: move |_| {
                            let n = states.read().len();
                            states.write().push(StateRow {
                                name: format!("state{n}"),
                                value: String::new(),
                                orig: None,
                            });
                        },
                        "+ Add state"
                    }
                    if remap_changed {
                        div { class: "rounded border p-2 text-xs text-amber-700",
                            "State changes will reshape this node's table and \
                             {child_count} child table(s)."
                        }
                    }
                }
            }
            if let Some(e) = error.read().clone() {
                div { class: "text-xs text-red-600", "{e}" }
            }
        }
    }
}
