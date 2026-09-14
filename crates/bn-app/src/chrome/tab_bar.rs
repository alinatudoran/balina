//! Tab bar: horizontal strip of tabs for switching between networks in the
//! project. Each tab shows its label, a modified indicator, and a close button.
//! Tabs can be dragged to reorder.

use bn_session::TabId;
use dioxus::prelude::*;

use crate::state::{exec, open_dialog, DialogDesc, SESSION};

/// Flush transient UI state from the outgoing tab and load state for the
/// incoming tab. Must be called AROUND `ops::tab::switch_tab`.
pub fn switch_to_tab(id: TabId) {
    use crate::canvas::controller::{
        DRAG_POS, FIT_REQUEST, Gesture, GESTURE, NOTE_DRAG_POS, NOTE_RESIZE, VIEWPORT,
    };
    use crate::state::{
        close_dialog, clear_selection, CONTEXT_MENU, EDITING_NOTE, TAB_VIEW_STATES,
    };

    let outgoing = SESSION.read().active_id();
    if outgoing == id {
        return;
    }

    // 1. Flush outgoing state
    {
        let vp = *VIEWPORT.read();
        let sel = crate::state::SELECTION.read().clone();
        let edit_note = *EDITING_NOTE.read();
        let mut tvs = TAB_VIEW_STATES.write();
        tvs.insert(outgoing, crate::state::TabViewState {
            viewport: vp,
            selection: sel,
            editing_note: edit_note,
        });
    }

    // 2. Clear transient gesture state
    *GESTURE.write() = Gesture::Idle;
    *DRAG_POS.write() = Default::default();
    *NOTE_DRAG_POS.write() = Default::default();
    *NOTE_RESIZE.write() = None;
    close_dialog();
    *CONTEXT_MENU.write() = None;

    // 3. Switch active tab in session
    exec(|s| { bn_session::ops::tab::switch_tab(s, id)?; Ok(()) });

    // 4. Load incoming state
    {
        let tvs = TAB_VIEW_STATES.read();
        if let Some(vs) = tvs.get(&id) {
            *VIEWPORT.write() = vs.viewport;
            *crate::state::SELECTION.write() = vs.selection.clone();
            *EDITING_NOTE.write() = vs.editing_note;
        } else {
            // First time visiting this tab — fit view
            clear_selection();
            *EDITING_NOTE.write() = None;
            *FIT_REQUEST.write() += 1;
        }
    }
}

pub fn add_new_tab() {
    exec(|s| { bn_session::ops::tab::add_tab(s); Ok(()) });
    *crate::canvas::controller::FIT_REQUEST.write() += 1;
}

pub fn close_tab(id: TabId) {
    // Closing: if this was the active tab, tab_bar will re-render and we
    // need to load the new active tab's viewport.
    let was_active = SESSION.read().active_id() == id;
    exec(|s| bn_session::ops::tab::close_tab(s, id));
    if was_active {
        let new_active = SESSION.read().active_id();
        let tvs = crate::state::TAB_VIEW_STATES.read();
        if let Some(vs) = tvs.get(&new_active) {
            *crate::canvas::controller::VIEWPORT.write() = vs.viewport;
            *crate::state::SELECTION.write() = vs.selection.clone();
            *crate::state::EDITING_NOTE.write() = vs.editing_note;
        } else {
            crate::state::clear_selection();
            *crate::state::EDITING_NOTE.write() = None;
            *crate::canvas::controller::FIT_REQUEST.write() += 1;
        }
    }
}

pub fn duplicate_tab(id: TabId) {
    exec(|s| bn_session::ops::tab::duplicate_tab(s, id));
    *crate::canvas::controller::FIT_REQUEST.write() += 1;
}

#[component]
pub fn TabBar() -> Element {
    let s = SESSION.read();
    let order = s.tab_order().to_vec();
    let active = s.active_id();
    // Collect tab info while we hold the read guard.
    let tabs: Vec<(TabId, String, bool)> = order
        .iter()
        .map(|&id| {
            let tab = s.tab(id);
            (id, tab.doc.net.name.clone(), tab.doc.modified)
        })
        .collect();
    drop(s);

    // Drag-to-reorder state: which tab is being dragged, and which index is
    // the current drop target (insert-before position).
    let mut dragging: Signal<Option<TabId>> = use_signal(|| None);
    let mut drag_over: Signal<Option<usize>> = use_signal(|| None);
    // Tab context menu: (tab id, client x, client y).
    let mut tab_menu: Signal<Option<(TabId, f64, f64)>> = use_signal(|| None);

    rsx! {
        div {
            class: "flex h-8 shrink-0 items-end gap-0 border-b bg-muted/40 px-1 overflow-x-auto",
            for (idx, (id, label, modified)) in tabs.iter().enumerate() {
                {
                    let id = *id;
                    let is_active = id == active;
                    let label = label.clone();
                    let modified = *modified;
                    let is_sep_active = dragging.read().is_some()
                        && drag_over.read().map_or(false, |t| t == idx);
                    let is_dragging_this = dragging.read().map_or(false, |d| d == id);

                    rsx! {
                        // Insertion-line separator before this tab.
                        div {
                            key: "sep-{idx}",
                            class: "flex items-center self-stretch",
                            ondragover: move |ev| {
                                ev.prevent_default();
                                if dragging.read().is_some() {
                                    drag_over.set(Some(idx));
                                }
                            },
                            ondrop: move |ev| {
                                ev.prevent_default();
                                let src = dragging.read().clone();
                                if let Some(src_id) = src {
                                    exec(|s| {
                                        bn_session::ops::tab::reorder_tab(s, src_id, idx);
                                        Ok(())
                                    });
                                }
                                dragging.set(None);
                                drag_over.set(None);
                            },
                            div {
                                class: if is_sep_active {
                                    "w-0.5 h-5 rounded bg-blue-500 mx-0.5"
                                } else {
                                    "w-px h-5 mx-0.5"
                                },
                            }
                        }
                        div {
                            draggable: "true",
                            class: if is_dragging_this {
                                if is_active {
                                    "group flex items-center gap-1 rounded-t border border-b-0 bg-background px-3 py-1 text-xs font-medium cursor-default opacity-50"
                                } else {
                                    "group flex items-center gap-1 rounded-t border border-transparent px-3 py-1 text-xs text-muted-foreground hover:bg-accent cursor-pointer opacity-50"
                                }
                            } else if is_active {
                                "group flex items-center gap-1 rounded-t border border-b-0 bg-background px-3 py-1 text-xs font-medium cursor-default"
                            } else {
                                "group flex items-center gap-1 rounded-t border border-transparent px-3 py-1 text-xs text-muted-foreground hover:bg-accent cursor-pointer"
                            },
                            onclick: move |_| {
                                if !is_active {
                                    switch_to_tab(id);
                                }
                            },
                            onauxclick: move |ev| {
                                if ev.data().trigger_button() == Some(dioxus::html::input_data::MouseButton::Auxiliary) {
                                    close_tab(id);
                                }
                            },
                            oncontextmenu: move |ev| {
                                ev.prevent_default();
                                let c = ev.data().client_coordinates();
                                tab_menu.set(Some((id, c.x, c.y)));
                            },
                            ondragstart: move |ev| {
                                ev.stop_propagation();
                                dragging.set(Some(id));
                                drag_over.set(None);
                            },
                            ondragover: move |ev| {
                                ev.prevent_default();
                                if dragging.read().is_some() {
                                    drag_over.set(Some(idx));
                                }
                            },
                            ondrop: move |ev| {
                                ev.prevent_default();
                                let src = dragging.read().clone();
                                if let Some(src_id) = src {
                                    if src_id != id {
                                        exec(|s| {
                                            bn_session::ops::tab::reorder_tab(s, src_id, idx);
                                            Ok(())
                                        });
                                    }
                                }
                                dragging.set(None);
                                drag_over.set(None);
                            },
                            ondragend: move |_| {
                                dragging.set(None);
                                drag_over.set(None);
                            },
                            span { "{label}" }
                            if modified {
                                span { class: "text-muted-foreground", " *" }
                            }
                            button {
                                class: "ml-1 inline-flex rounded p-0.5 text-muted-foreground hover:bg-red-100 hover:text-red-600",
                                onclick: move |ev| {
                                    ev.stop_propagation();
                                    close_tab(id);
                                },
                                "×"
                            }
                        }
                    }
                }
            }
            // Trailing separator (drop target for "append to end").
            {
                let tab_count = tabs.len();
                let is_last_sep_active = dragging.read().is_some()
                    && drag_over.read().map_or(false, |t| t == tab_count);
                rsx! {
                    div {
                        class: "flex items-center self-stretch",
                        ondragover: move |ev| {
                            ev.prevent_default();
                            if dragging.read().is_some() {
                                drag_over.set(Some(tab_count));
                            }
                        },
                        ondrop: move |ev| {
                            ev.prevent_default();
                            let src = dragging.read().clone();
                            if let Some(src_id) = src {
                                exec(|s| {
                                    bn_session::ops::tab::reorder_tab(s, src_id, tab_count);
                                    Ok(())
                                });
                            }
                            dragging.set(None);
                            drag_over.set(None);
                        },
                        div {
                            class: if is_last_sep_active {
                                "w-0.5 h-5 rounded bg-blue-500 mx-0.5"
                            } else {
                                "w-px h-5 mx-0.5"
                            },
                        }
                    }
                }
            }
            button {
                class: "flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-accent mb-0.5",
                onclick: move |_| add_new_tab(),
                title: "New tab",
                "+"
            }
        }
        // Tab context menu popup.
        if let Some((menu_id, mx, my)) = *tab_menu.read() {
            {
                const ITEM: &str = "flex w-full cursor-default items-center px-3 py-1.5 text-left \
                                    text-xs hover:bg-accent rounded-sm";
                rsx! {
                    // Click-catcher to close the menu.
                    div {
                        class: "fixed inset-0 z-40",
                        onclick: move |_| tab_menu.set(None),
                        oncontextmenu: move |ev| { ev.prevent_default(); tab_menu.set(None); },
                    }
                    div {
                        class: "fixed z-50 min-w-36 rounded-md border bg-popover p-1 \
                                text-popover-foreground shadow-md",
                        style: "left: {mx}px; top: {my}px;",
                        button {
                            class: ITEM,
                            onclick: move |_| {
                                tab_menu.set(None);
                                open_dialog(DialogDesc::RenameTab { id: menu_id });
                            },
                            "Rename…"
                        }
                        button {
                            class: ITEM,
                            onclick: move |_| {
                                tab_menu.set(None);
                                duplicate_tab(menu_id);
                            },
                            "Duplicate"
                        }
                        div { class: "my-1 border-t" }
                        button {
                            class: ITEM,
                            onclick: move |_| {
                                tab_menu.set(None);
                                // Close all other tabs.
                                let ids: Vec<TabId> = SESSION.read()
                                    .tab_order()
                                    .iter()
                                    .copied()
                                    .filter(|&t| t != menu_id)
                                    .collect();
                                for t in ids {
                                    close_tab(t);
                                }
                            },
                            "Close Others"
                        }
                        button {
                            class: "flex w-full cursor-default items-center px-3 py-1.5 text-left \
                                    text-xs hover:bg-red-100 hover:text-red-600 rounded-sm",
                            onclick: move |_| {
                                tab_menu.set(None);
                                close_tab(menu_id);
                            },
                            "Close"
                        }
                    }
                }
            }
        }
    }
}
