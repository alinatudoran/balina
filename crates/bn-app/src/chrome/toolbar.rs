//! Toolbar: node palette (click to add at view center, drag onto canvas),
//! compile, auto-update, fit view, zoom display.

use bn_core::model::NodeKind;
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;
use icons::{Diamond, Dices, SquareSplitVertical};

use crate::canvas::canvas::default_node_footprint;
use crate::canvas::controller::{self, Gesture, FIT_REQUEST, GESTURE, VIEWPORT};
use crate::state::{exec, SESSION};

const BTN: &str = "inline-flex h-7 cursor-default items-center gap-1 rounded-md border \
                   bg-background px-2 text-xs shadow-xs hover:bg-accent";

pub const PALETTE: [(NodeKind, &str); 3] = [
    (NodeKind::Chance, "Chance"),
    (NodeKind::Decision, "Decision"),
    (NodeKind::Utility, "Utility"),
];

fn palette_icon(kind: NodeKind) -> Element {
    let class = Some("size-3.5".to_string());
    match kind {
        NodeKind::Chance => rsx! { Dices { class } },
        NodeKind::Decision => rsx! { SquareSplitVertical { class } },
        NodeKind::Utility => rsx! { Diamond { class } },
    }
}

/// Add a node of `kind` centered in the current view.
pub fn add_at_center(kind: NodeKind) {
    let vp = *VIEWPORT.read();
    let c = vp.element_to_world(vp.size.0 / 2.0, vp.size.1 / 2.0);
    let (w, h) = default_node_footprint(kind);
    exec(|s| {
        bn_session::ops::edit::add_node(s, kind, (c.x - w / 2.0) as f32, (c.y - h / 2.0) as f32)
    });
}

#[component]
pub fn Toolbar() -> Element {
    let auto_update = SESSION.read().doc.auto_update;
    let zoom_pct = (VIEWPORT.read().zoom * 100.0).round() as i32;

    rsx! {
        div { class: "flex shrink-0 items-center gap-2 border-b px-2 py-1",
            span { class: "text-xs text-muted-foreground", "Add:" }
            for (kind, label) in PALETTE {
                button {
                    class: "{BTN} cursor-grab",
                    title: "Click to add at view center, or drag onto the canvas",
                    onmousedown: move |ev| {
                        if ev.data().trigger_button() != Some(MouseButton::Primary) {
                            return;
                        }
                        ev.stop_propagation();
                        let c = ev.data().client_coordinates();
                        *GESTURE.write() = Gesture::PaletteDrag {
                            kind,
                            start_client: (c.x, c.y),
                            cur_client: (c.x, c.y),
                        };
                    },
                    {palette_icon(kind)}
                    "{label}"
                }
            }
            div { class: "h-5 w-px bg-border" }
            button {
                class: BTN,
                onclick: move |_| {
                    exec(|s| {
                        bn_session::ops::edit::recompute(s);
                        Ok(())
                    });
                },
                "⚡ Compile"
            }
            div { class: "flex items-center gap-1.5",
                input {
                    id: "auto-update",
                    r#type: "checkbox",
                    checked: auto_update,
                    onchange: move |ev| {
                        let on = ev.checked();
                        exec(|s| {
                            bn_session::ops::edit::set_auto_update(s, on);
                            Ok(())
                        });
                        crate::chrome::menu::sync_auto_update_item(on);
                    },
                }
                label { r#for: "auto-update", class: "text-xs", "Auto update" }
            }
            div { class: "h-5 w-px bg-border" }
            button {
                class: BTN,
                onclick: move |_| {
                    *FIT_REQUEST.write() += 1;
                },
                "⛶ Fit view"
            }
            div { class: "ml-auto flex items-center gap-1",
                button {
                    class: BTN,
                    title: "Zoom out (Cmd/Ctrl+-)",
                    onclick: move |_| controller::zoom_center(1.0 / 1.25),
                    "−"
                }
                button {
                    class: "w-12 rounded-md px-1 text-center text-xs text-muted-foreground hover:bg-accent",
                    title: "Reset zoom to 100% (Cmd/Ctrl+0)",
                    onclick: move |_| controller::zoom_reset(),
                    "{zoom_pct}%"
                }
                button {
                    class: BTN,
                    title: "Zoom in (Cmd/Ctrl+=)",
                    onclick: move |_| controller::zoom_center(1.25),
                    "+"
                }
            }
        }
    }
}
