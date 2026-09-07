//! In-app menu bar for the web build (browsers have no native menu). Every
//! item routes through `chrome::menu::route` by the same id strings the
//! native muda menu uses, so the two menus can never drift apart. On desktop
//! the component renders nothing.

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use dioxus::prelude::*;

    /// Desktop has the native muda menu; render nothing.
    #[component]
    pub fn MenuBar() -> Element {
        rsx! {}
    }
}

pub use imp::MenuBar;

#[cfg(target_arch = "wasm32")]
mod imp {
    use dioxus::prelude::*;

    use crate::state::SESSION;

    struct Item {
        id: &'static str,
        label: &'static str,
        /// Shown as a hint; the shortcut itself lives in `chrome::hotkeys`.
        accel: Option<&'static str>,
        sep_before: bool,
        /// Renders a checkmark from `doc.auto_update`.
        auto_update_check: bool,
    }

    const fn item(id: &'static str, label: &'static str) -> Item {
        Item { id, label, accel: None, sep_before: false, auto_update_check: false }
    }
    const fn item_accel(id: &'static str, label: &'static str, accel: &'static str) -> Item {
        Item { id, label, accel: Some(accel), sep_before: false, auto_update_check: false }
    }
    const fn item_sep(id: &'static str, label: &'static str) -> Item {
        Item { id, label, accel: None, sep_before: true, auto_update_check: false }
    }

    const MENUS: &[(&str, &[Item])] = &[
        (
            "File",
            &[
                item_accel("file.new", "New", "⌘N"),
                item_accel("file.open", "Open…", "⌘O"),
                item_accel("file.save", "Save…", "⌘S"),
                Item {
                    id: "file.saveAs",
                    label: "Save As…",
                    accel: Some("⇧⌘S"),
                    sep_before: false,
                    auto_update_check: false,
                },
                item_sep("file.exportSvg", "Export as SVG"),
                item("file.exportPng", "Export as PNG"),
            ],
        ),
        (
            "Edit",
            &[
                item_accel("edit.undo", "Undo", "⌘Z"),
                item_accel("edit.redo", "Redo", "⇧⌘Z"),
                item_sep("edit.selectAll", "Select All Nodes"),
                item_accel("edit.deleteSelection", "Delete Selection", "⌫"),
            ],
        ),
        (
            "Network",
            &[
                item_accel("network.compile", "Compile Now", "F5"),
                Item {
                    id: "network.autoUpdate",
                    label: "Auto Update Beliefs",
                    accel: None,
                    sep_before: false,
                    auto_update_check: true,
                },
                item_sep("network.removeFindings", "Remove All Findings"),
                item("network.rename", "Network Name…"),
            ],
        ),
        (
            "Cases",
            &[
                item("cases.learnCpts", "Learn CPTs from Cases…"),
                item("cases.learnStructure", "Learn Structure from Cases…"),
                item("cases.simulate", "Simulate Cases…"),
            ],
        ),
        (
            "Tools",
            &[
                item("tools.sensitivity", "Sensitivity to Findings…"),
                item("tools.arcStrength", "Arc Strengths…"),
                item("tools.solveId", "Solve Influence Diagram"),
                item_sep("help.about", "About Balina"),
            ],
        ),
    ];

    const MENU_BTN: &str = "rounded px-2 py-0.5 text-sm hover:bg-accent";
    const MENU_BTN_OPEN: &str = "rounded px-2 py-0.5 text-sm bg-accent";
    const MENU_ITEM: &str = "flex w-full cursor-default items-center justify-between gap-6 \
                             rounded-sm px-2 py-1.5 text-left text-sm hover:bg-accent";

    #[component]
    pub fn MenuBar() -> Element {
        let mut open: Signal<Option<usize>> = use_signal(|| None);
        let auto_update = SESSION.read().doc.auto_update;
        let is_open = *open.read();

        rsx! {
            // z-50 keeps the whole bar (titles + dropdowns) above the z-40
            // click-catcher so hover-switching between open menus works.
            div { class: "relative z-50 flex shrink-0 items-center gap-0.5 border-b bg-background px-2 py-1",
                for (i, (title, items)) in MENUS.iter().enumerate() {
                    div { class: "relative",
                        button {
                            class: if is_open == Some(i) { MENU_BTN_OPEN } else { MENU_BTN },
                            onclick: move |_| {
                                let cur = *open.peek();
                                open.set(if cur == Some(i) { None } else { Some(i) });
                            },
                            onmouseenter: move |_| {
                                // Menu-bar convention: once one menu is open,
                                // hovering moves between menus.
                                if open.peek().is_some() {
                                    open.set(Some(i));
                                }
                            },
                            "{title}"
                        }
                        if is_open == Some(i) {
                            div {
                                class: "absolute top-full left-0 z-50 mt-1 w-64 rounded-md border \
                                        bg-popover p-1 text-popover-foreground shadow-md",
                                for it in items.iter() {
                                    if it.sep_before {
                                        div { class: "my-1 border-t" }
                                    }
                                    button {
                                        class: MENU_ITEM,
                                        onclick: {
                                            let id = it.id;
                                            move |_| {
                                                open.set(None);
                                                crate::chrome::menu::route(id);
                                            }
                                        },
                                        span {
                                            if it.auto_update_check {
                                                if auto_update { "✓ {it.label}" } else { "{it.label}" }
                                            } else {
                                                "{it.label}"
                                            }
                                        }
                                        if let Some(a) = it.accel {
                                            span { class: "text-xs text-muted-foreground", "{a}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Click-catcher: closes the open dropdown on any outside click.
            if is_open.is_some() {
                div {
                    class: "fixed inset-0 z-40",
                    onclick: move |_| open.set(None),
                }
            }
        }
    }
}
