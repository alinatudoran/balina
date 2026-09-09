//! Root component: layout, menu-event routing, window-title sync, initial
//! file open, palette ghost-drag handling.

use dioxus::prelude::*;

use crate::canvas::context_menu::ContextMenuHost;
use crate::canvas::controller::{Gesture, PaletteItem, GESTURE, VIEWPORT};
use crate::canvas::node::kind_header_color;
use crate::canvas::sticky::note_fill;
use crate::chrome::toolbar::{add_at_center, palette_footprint, Toolbar};
use crate::chrome::{message_log::MessageLog, status_bar::StatusBar};
use crate::state::{exec, SESSION};

/// Finish a palette drag at `c` (client coords): a barely-moved click adds
/// at view center; a drop inside the canvas adds at the cursor; anything
/// else cancels.
fn finish_palette_drag(item: PaletteItem, start: (f64, f64), c: (f64, f64)) {
    *GESTURE.write() = Gesture::Idle;
    let moved = ((c.0 - start.0).powi(2) + (c.1 - start.1).powi(2)).sqrt() > 4.0;
    if !moved {
        add_at_center(item);
        return;
    }
    let vp = *VIEWPORT.read();
    let inside = c.0 >= vp.origin.0
        && c.0 <= vp.origin.0 + vp.size.0
        && c.1 >= vp.origin.1
        && c.1 <= vp.origin.1 + vp.size.1;
    if inside {
        let w = vp.client_to_world(c.0, c.1);
        let (fw, fh) = palette_footprint(item);
        let (x, y) = ((w.x - fw / 2.0) as f32, (w.y - fh / 2.0) as f32);
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
}

#[component]
pub fn App() -> Element {
    // Initial mount: open the CLI file (desktop) / the `?file={url}` network
    // (web), or make sure beliefs exist.
    use_hook(|| {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = crate::INITIAL_FILE.get() {
            crate::chrome::file_ops::open_path(path.clone());
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(url) = crate::platform::initial_file_url() {
            crate::chrome::file_ops::open_url(url);
            return;
        }
        exec(|s| {
            bn_session::ops::file::refresh(s);
            Ok(())
        });
    });

    // Native menu events → route by id.
    #[cfg(not(target_arch = "wasm32"))]
    dioxus::desktop::use_muda_event_handler(move |ev| {
        crate::chrome::menu::route(ev.id().as_ref());
    });

    // Window title tracks name + modified star.
    use_effect(move || {
        let s = SESSION.read();
        let title =
            format!("Balina — {}{}", s.doc.net.name, if s.doc.modified { " *" } else { "" });
        drop(s);
        crate::platform::set_window_title(&title);
    });

    let palette_ghost = match &*GESTURE.read() {
        Gesture::PaletteDrag { item, cur_client, .. } => Some((*item, *cur_client)),
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
                if let Gesture::PaletteDrag { item, start_client, .. } = g {
                    let c = ev.data().client_coordinates();
                    if ev.data().held_buttons().is_empty() {
                        finish_palette_drag(item, start_client, (c.x, c.y));
                    } else {
                        *GESTURE.write() = Gesture::PaletteDrag {
                            item,
                            start_client,
                            cur_client: (c.x, c.y),
                        };
                    }
                }
            },
            onmouseup: move |ev| {
                let g = GESTURE.read().clone();
                if let Gesture::PaletteDrag { item, start_client, .. } = g {
                    let c = ev.data().client_coordinates();
                    finish_palette_drag(item, start_client, (c.x, c.y));
                }
            },
            crate::chrome::menu_bar::MenuBar {}
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
            if let Some((item, (gx, gy))) = palette_ghost {
                {
                    let (bg, label) = match item {
                        PaletteItem::Node(kind) => {
                            (kind_header_color(kind).to_string(), format!("{kind:?}"))
                        }
                        PaletteItem::Note => {
                            (note_fill(bn_session::doc::NOTE_DEFAULT_COLOR), "Note".to_string())
                        }
                    };
                    rsx! {
                        div {
                            class: "pointer-events-none fixed z-50 rounded border px-2 py-1 text-xs shadow-md",
                            style: "left: {gx + 6.0}px; top: {gy + 6.0}px; background: {bg};",
                            "{label}"
                        }
                    }
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
