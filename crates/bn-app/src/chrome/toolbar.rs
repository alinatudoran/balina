//! Toolbar: node palette (click to add at view center, drag onto canvas),
//! compile, auto-update, fit view, zoom display.

use bn_core::model::NodeKind;
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;
use icons::{Diamond, Dices, SquareSplitVertical, StickyNote};

use crate::canvas::canvas::default_node_footprint;
use crate::canvas::controller::{self, Gesture, PaletteItem, FIT_REQUEST, GESTURE, VIEWPORT};
use crate::state::{exec, SESSION};

const BTN: &str = "inline-flex h-7 cursor-default items-center gap-1 rounded-md border \
                   bg-background px-2 text-xs shadow-xs hover:bg-accent";

pub const PALETTE: [(PaletteItem, &str); 4] = [
    (PaletteItem::Node(NodeKind::Chance), "Chance"),
    (PaletteItem::Node(NodeKind::Decision), "Decision"),
    (PaletteItem::Node(NodeKind::Utility), "Utility"),
    (PaletteItem::Note, "Note"),
];

fn palette_icon(item: PaletteItem) -> Element {
    let class = Some("size-3.5".to_string());
    match item {
        PaletteItem::Node(NodeKind::Chance) => rsx! { Dices { class } },
        PaletteItem::Node(NodeKind::Decision) => rsx! { SquareSplitVertical { class } },
        PaletteItem::Node(NodeKind::Utility) => rsx! { Diamond { class } },
        PaletteItem::Note => rsx! { StickyNote { class } },
    }
}

/// Footprint used to center palette drops before the item exists.
pub fn palette_footprint(item: PaletteItem) -> (f64, f64) {
    match item {
        PaletteItem::Node(kind) => default_node_footprint(kind),
        PaletteItem::Note => {
            (bn_session::NOTE_DEFAULT_SIZE.0 as f64, bn_session::NOTE_DEFAULT_SIZE.1 as f64)
        }
    }
}

/// Add a palette item centered in the current view.
pub fn add_at_center(item: PaletteItem) {
    let vp = *VIEWPORT.read();
    let c = vp.element_to_world(vp.size.0 / 2.0, vp.size.1 / 2.0);
    let (w, h) = palette_footprint(item);
    let (x, y) = ((c.x - w / 2.0) as f32, (c.y - h / 2.0) as f32);
    match item {
        PaletteItem::Node(kind) => {
            exec(|s| bn_session::ops::edit::add_node(s, kind, x, y));
        }
        PaletteItem::Note => {
            if let Some(id) = exec(|s| bn_session::ops::edit::add_note(s, x, y)) {
                crate::state::focus_new_note(id);
            }
        }
    }
}

#[component]
pub fn Toolbar() -> Element {
    let auto_update = SESSION.read().doc.auto_update;
    let zoom_pct = (VIEWPORT.read().zoom * 100.0).round() as i32;

    rsx! {
        div { class: "flex shrink-0 items-center gap-2 border-b px-2 py-1",
            span { class: "text-xs text-muted-foreground", "Add:" }
            for (item, label) in PALETTE {
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
                            item,
                            start_client: (c.x, c.y),
                            cur_client: (c.x, c.y),
                        };
                    },
                    {palette_icon(item)}
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
