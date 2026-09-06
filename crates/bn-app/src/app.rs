//! Root component: layout, menu-event routing, window-title sync, initial
//! file open, palette ghost-drag handling.

use dioxus::desktop::use_muda_event_handler;
use dioxus::prelude::*;

use crate::canvas::canvas::default_node_footprint;
use crate::canvas::context_menu::ContextMenuHost;
use crate::canvas::controller::{Gesture, GESTURE, VIEWPORT};
use crate::canvas::node::kind_header_color;
use crate::chrome::toolbar::{add_at_center, Toolbar};
use crate::chrome::{menu, message_log::MessageLog, status_bar::StatusBar};
use crate::state::{exec, SESSION};

/// Finish a palette drag at `c` (client coords): a barely-moved click adds
/// at view center; a drop inside the canvas adds at the cursor; anything
/// else cancels.
fn finish_palette_drag(kind: bn_core::model::NodeKind, start: (f64, f64), c: (f64, f64)) {
    *GESTURE.write() = Gesture::Idle;
    let moved = ((c.0 - start.0).powi(2) + (c.1 - start.1).powi(2)).sqrt() > 4.0;
    if !moved {
        add_at_center(kind);
        return;
    }
    let vp = *VIEWPORT.read();
    let inside = c.0 >= vp.origin.0
        && c.0 <= vp.origin.0 + vp.size.0
        && c.1 >= vp.origin.1
        && c.1 <= vp.origin.1 + vp.size.1;
    if inside {
        let w = vp.client_to_world(c.0, c.1);
        let (fw, fh) = default_node_footprint(kind);
        exec(|s| {
            bn_session::ops::edit::add_node(
                s,
                kind,
                (w.x - fw / 2.0) as f32,
                (w.y - fh / 2.0) as f32,
            )
        });
    }
}

#[component]
pub fn App() -> Element {
    // Initial mount: open the CLI file or make sure beliefs exist.
    use_hook(|| {
        if let Some(path) = crate::INITIAL_FILE.get() {
            crate::chrome::file_ops::open_path(path.clone());
        } else {
            exec(|s| {
                bn_session::ops::file::refresh(s);
                Ok(())
            });
        }
    });

    // Native menu events → route by id.
    use_muda_event_handler(move |ev| {
        menu::route(ev.id().as_ref());
    });

    // Window title tracks name + modified star.
    use_effect(move || {
        let s = SESSION.read();
        let title =
            format!("Balina — {}{}", s.doc.net.name, if s.doc.modified { " *" } else { "" });
        drop(s);
        dioxus::desktop::window().set_title(&title);
    });

    let palette_ghost = match &*GESTURE.read() {
        Gesture::PaletteDrag { kind, cur_client, .. } => Some((*kind, *cur_client)),
        _ => None,
    };
    let conflict = SESSION.read().bridge.conflict;

    rsx! {
        Stylesheet {}
        div {
            class: "flex h-screen flex-col bg-background text-foreground outline-none",
            tabindex: "0",
            autofocus: true,
            onkeydown: crate::chrome::hotkeys::handle_keydown,
            onmousemove: move |ev| {
                // Bind the clone FIRST: an `if let … = GESTURE.read().clone()`
                // keeps the read guard alive through the body (Rust 2024
                // scrutinee scoping), and the body writes GESTURE → panic.
                let g = GESTURE.read().clone();
                if let Gesture::PaletteDrag { kind, start_client, .. } = g {
                    let c = ev.data().client_coordinates();
                    if ev.data().held_buttons().is_empty() {
                        finish_palette_drag(kind, start_client, (c.x, c.y));
                    } else {
                        *GESTURE.write() = Gesture::PaletteDrag {
                            kind,
                            start_client,
                            cur_client: (c.x, c.y),
                        };
                    }
                }
            },
            onmouseup: move |ev| {
                let g = GESTURE.read().clone();
                if let Gesture::PaletteDrag { kind, start_client, .. } = g {
                    let c = ev.data().client_coordinates();
                    finish_palette_drag(kind, start_client, (c.x, c.y));
                }
            },
            Toolbar {}
            if conflict {
                div {
                    class: "flex shrink-0 items-center gap-3 border-b border-red-200 \
                            bg-red-50 px-3 py-1.5 text-xs text-red-800",
                    span { class: "font-medium",
                        "⚠ The findings are contradictory (P(evidence) = 0) — beliefs cannot be updated."
                    }
                    button {
                        class: "rounded border border-red-300 bg-white px-2 py-0.5 hover:bg-red-100",
                        onclick: move |_| {
                            exec(|s| {
                                bn_session::ops::evidence::retract_all_findings(s);
                                Ok(())
                            });
                        },
                        "Remove all findings"
                    }
                }
            }
            div { class: "min-h-0 flex-1",
                crate::canvas::canvas::NetworkCanvas {}
            }
            StatusBar {}
            MessageLog {}
            ContextMenuHost {}
            crate::dialogs::host::DialogHost {}
            if let Some((kind, (gx, gy))) = palette_ghost {
                div {
                    class: "pointer-events-none fixed z-50 rounded border px-2 py-1 text-xs shadow-md",
                    style: "left: {gx + 6.0}px; top: {gy + 6.0}px; background: {kind_header_color(kind)};",
                    "{kind:?}"
                }
            }
        }
    }
}

/// Isolated so the (static) stylesheet element never re-renders — diffing
/// `document::Style` props is unsupported and warns.
#[component]
fn Stylesheet() -> Element {
    rsx! {
        document::Style { {include_str!("../assets/main.css")} }
    }
}
