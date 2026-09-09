//! Right-click menus for nodes, edges, and the canvas background — a fixed
//! panel at the click point over a full-screen click-catcher (the overlay
//! pattern from the upstream workflow component).

use bn_core::io::DisplayMode;
use bn_core::model::{NodeId, NodeKind};
use bn_session::NoteId;
use dioxus::prelude::*;

use crate::canvas::controller::FIT_REQUEST;
use crate::canvas::sticky::{note_fill, NOTE_PALETTE};
use crate::state::{
    exec, open_dialog, ContextMenuTarget, DialogDesc, CONTEXT_MENU, SESSION,
};

const ITEM: &str = "flex w-full cursor-default items-center rounded-sm px-2 py-1.5 text-left \
                    text-sm hover:bg-accent";
const ITEM_DANGER: &str = "flex w-full cursor-default items-center rounded-sm px-2 py-1.5 \
                           text-left text-sm text-destructive hover:bg-destructive/10";

fn close() {
    *CONTEXT_MENU.write() = None;
}

#[component]
pub fn ContextMenuHost() -> Element {
    let Some(menu) = CONTEXT_MENU.read().clone() else { return rsx! {} };
    let (cx, cy) = menu.client;

    rsx! {
        div {
            class: "fixed inset-0 z-40",
            onclick: move |_| close(),
            oncontextmenu: move |ev| {
                ev.prevent_default();
                close();
            },
        }
        div {
            class: "fixed z-50 w-56 rounded-md border bg-popover p-1 text-popover-foreground shadow-md",
            style: "left: {cx}px; top: {cy}px;",
            onclick: move |ev| ev.stop_propagation(),
            oncontextmenu: move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
            },
            match menu.target {
                ContextMenuTarget::Node(id) => rsx! {
                    NodeMenu { id }
                },
                ContextMenuTarget::Edge(parent, child) => rsx! {
                    button {
                        class: ITEM_DANGER,
                        onclick: move |_| {
                            close();
                            exec(|s| bn_session::ops::edit::remove_edge(s, parent, child));
                        },
                        "Delete link"
                    }
                },
                ContextMenuTarget::Note(id) => rsx! {
                    NoteMenu { id }
                },
                ContextMenuTarget::Pane { world } => rsx! {
                    PaneMenu { world }
                },
            }
        }
    }
}

#[component]
fn NodeMenu(id: NodeId) -> Element {
    let (kind, display, has_finding) = {
        let s = SESSION.read();
        if !s.doc.net.contains(id) {
            return rsx! {};
        }
        (
            s.doc.net.node(id).kind,
            s.doc.visual.get(id).map(|v| v.display).unwrap_or_default(),
            s.doc.evidence.get(id).is_some(),
        )
    };

    rsx! {
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                open_dialog(DialogDesc::NodeProperties { node: id });
            },
            "Properties…"
        }
        if kind != NodeKind::Decision {
            button {
                class: ITEM,
                onclick: move |_| {
                    close();
                    open_dialog(DialogDesc::CptEditor { node: id });
                },
                if kind == NodeKind::Utility { "Utility table…" } else { "CPT…" }
            }
        }
        div { class: "my-1 border-t" }
        span { class: "px-2 text-xs text-muted-foreground", "Display as" }
        for (mode, label) in [
            (DisplayMode::BeliefBars, "Belief bars"),
            (DisplayMode::TitleOnly, "Title only"),
            (DisplayMode::ExpectedValue, "Expected value"),
        ] {
            button {
                class: ITEM,
                onclick: move |_| {
                    close();
                    exec(|s| bn_session::ops::edit::set_display_mode(s, id, mode));
                },
                span { class: "w-4", if display == mode { "•" } else { "" } }
                "{label}"
            }
        }
        if kind == NodeKind::Chance {
            div { class: "my-1 border-t" }
            button {
                class: ITEM,
                onclick: move |_| {
                    close();
                    open_dialog(DialogDesc::Likelihood { node: id });
                },
                "Likelihood finding…"
            }
        }
        if has_finding {
            button {
                class: ITEM,
                onclick: move |_| {
                    close();
                    exec(|s| bn_session::ops::evidence::retract_finding(s, id));
                },
                "Remove finding"
            }
        }
        div { class: "my-1 border-t" }
        button {
            class: ITEM_DANGER,
            onclick: move |_| {
                close();
                exec(|s| bn_session::ops::edit::delete_items(s, &[id], &[], &[]));
            },
            "Delete"
        }
    }
}

#[component]
fn NoteMenu(id: NoteId) -> Element {
    let current = {
        let s = SESSION.read();
        let Some(n) = s.doc.notes.get(id) else { return rsx! {} };
        n.color
    };

    let swatches: Vec<([u8; 3], &str, String)> = NOTE_PALETTE
        .iter()
        .map(|&(color, label)| {
            let (bc, bw) = if color == current {
                ("rgb(100,140,220)", "2px")
            } else {
                ("rgba(0,0,0,0.2)", "1px")
            };
            let style = format!(
                "background: {}; border-color: {bc}; border-width: {bw};",
                note_fill(color)
            );
            (color, label, style)
        })
        .collect();

    rsx! {
        span { class: "px-2 text-xs text-muted-foreground", "Color" }
        div { class: "flex items-center gap-1.5 px-2 py-1.5",
            for (color, label, style) in swatches {
                button {
                    class: "size-6 rounded-full border hover:scale-110",
                    style,
                    title: "{label}",
                    onclick: move |_| {
                        close();
                        exec(|s| bn_session::ops::edit::set_note_color(s, id, color));
                    },
                }
            }
        }
        div { class: "my-1 border-t" }
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                exec(|s| bn_session::ops::edit::nudge_note_font(s, id, -1.0));
            },
            "Smaller text"
        }
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                exec(|s| bn_session::ops::edit::nudge_note_font(s, id, 1.0));
            },
            "Larger text"
        }
        div { class: "my-1 border-t" }
        button {
            class: ITEM_DANGER,
            onclick: move |_| {
                close();
                exec(|s| bn_session::ops::edit::delete_items(s, &[], &[], &[id]));
            },
            "Delete"
        }
    }
}

#[component]
fn PaneMenu(world: (f64, f64)) -> Element {
    rsx! {
        for (kind, label) in [
            (NodeKind::Chance, "Add chance node"),
            (NodeKind::Decision, "Add decision node"),
            (NodeKind::Utility, "Add utility node"),
        ] {
            button {
                class: ITEM,
                onclick: move |_| {
                    close();
                    exec(|s| {
                        bn_session::ops::edit::add_node(s, kind, world.0 as f32, world.1 as f32)
                    });
                },
                "{label}"
            }
        }
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                if let Some(id) =
                    exec(|s| bn_session::ops::edit::add_note(s, world.0 as f32, world.1 as f32))
                {
                    crate::state::focus_new_note(id);
                }
            },
            "Add note"
        }
        div { class: "my-1 border-t" }
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                exec(|s| {
                    bn_session::ops::evidence::retract_all_findings(s);
                    Ok(())
                });
            },
            "Remove all findings"
        }
        button {
            class: ITEM,
            onclick: move |_| {
                close();
                *FIT_REQUEST.write() += 1;
            },
            "Fit view"
        }
    }
}
