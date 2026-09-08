//! Native menu bar (muda — the same crate the Tauri shell used underneath).
//! Every item routes through [`route`] by id string.
//!
//! Accelerator ownership rule: a shortcut is either a native accelerator
//! (Cmd+N/O/S, Cmd+Shift+S, F5 — safe while typing) or a frontend hotkey
//! (Cmd+Z, Cmd+A, Delete — need the not-while-typing guard), never both.
//! The predefined clipboard items are required on macOS: without them
//! Cmd+C/V/X do not work inside webview text inputs at all.

use dioxus::prelude::ReadableExt;

use crate::state::{self, exec, open_dialog, DialogDesc, SESSION};

#[cfg(not(target_arch = "wasm32"))]
use dioxus::desktop::muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// Handle to the Auto Update check item so `sync_menu_state` can flip it.
    static AUTO_UPDATE_ITEM: std::cell::RefCell<Option<CheckMenuItem>> =
        const { std::cell::RefCell::new(None) };
    /// Handle to the Open Recent submenu so the list can be rebuilt live.
    static RECENT_SUBMENU: std::cell::RefCell<Option<Submenu>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(not(target_arch = "wasm32"))]
fn item(id: &str, text: &str) -> MenuItem {
    MenuItem::with_id(id, text, true, None)
}

#[cfg(not(target_arch = "wasm32"))]
fn item_accel(id: &str, text: &str, accel: &str) -> MenuItem {
    MenuItem::with_id(id, text, true, Some(accel.parse().expect("valid accelerator")))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn build() -> Menu {
    let menu = Menu::new();

    #[cfg(target_os = "macos")]
    {
        let app_menu = Submenu::new("Balina", true);
        app_menu
            .append_items(&[
                &item("help.about", "About Balina"),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::quit(None),
            ])
            .unwrap();
        menu.append(&app_menu).unwrap();
    }

    let recent = Submenu::new("Open Recent", true);
    RECENT_SUBMENU.with(|c| *c.borrow_mut() = Some(recent.clone()));
    rebuild_recent_items(&crate::chrome::recent::load());

    let file = Submenu::new("&File", true);
    file.append_items(&[
        &item_accel("file.new", "New", "CmdOrCtrl+N"),
        &item_accel("file.open", "Open…", "CmdOrCtrl+O"),
        &recent,
        &PredefinedMenuItem::separator(),
        &item_accel("file.save", "Save", "CmdOrCtrl+S"),
        &item_accel("file.saveAs", "Save As… (balina / xmlbif / xdsl)", "CmdOrCtrl+Shift+S"),
        &PredefinedMenuItem::separator(),
        &item("file.exportSvg", "Export as SVG…"),
        &item("file.exportPng", "Export as PNG…"),
    ])
    .unwrap();
    #[cfg(not(target_os = "macos"))]
    file.append_items(&[&PredefinedMenuItem::separator(), &PredefinedMenuItem::quit(None)])
        .unwrap();

    // Undo/Redo/Select All/Delete get NO accelerators: the frontend owns
    // Cmd+Z/Cmd+A/Delete with a not-while-typing guard, so text inputs keep
    // their native editing shortcuts.
    let edit = Submenu::new("&Edit", true);
    edit.append_items(&[
        &item("edit.undo", "Undo"),
        &item("edit.redo", "Redo"),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::separator(),
        &item("edit.selectAll", "Select All Nodes"),
        &item("edit.deleteSelection", "Delete Selection"),
    ])
    .unwrap();

    let auto_update = CheckMenuItem::with_id(
        "network.autoUpdate",
        "Auto Update Beliefs",
        true,
        true,
        None,
    );
    let network = Submenu::new("&Network", true);
    network
        .append_items(&[
            &item_accel("network.compile", "Compile Now", "F5"),
            &auto_update,
            &PredefinedMenuItem::separator(),
            &item("network.removeFindings", "Remove All Findings"),
            &item("network.rename", "Network Name…"),
        ])
        .unwrap();
    AUTO_UPDATE_ITEM.with(|c| *c.borrow_mut() = Some(auto_update));

    let cases = Submenu::new("&Cases", true);
    cases
        .append_items(&[
            &item("cases.learnCpts", "Learn CPTs from Cases…"),
            &item("cases.learnStructure", "Learn Structure from Cases…"),
            &item("cases.simulate", "Simulate Cases…"),
        ])
        .unwrap();

    let tools = Submenu::new("&Tools", true);
    tools
        .append_items(&[
            &item("tools.sensitivity", "Sensitivity to Findings…"),
            &item("tools.arcStrength", "Arc Strengths…"),
            &item("tools.solveId", "Solve Influence Diagram"),
        ])
        .unwrap();

    menu.append_items(&[&file, &edit, &network, &cases, &tools]).unwrap();

    #[cfg(not(target_os = "macos"))]
    {
        let help = Submenu::new("&Help", true);
        help.append(&item("help.about", "About Balina")).unwrap();
        menu.append(&help).unwrap();
    }

    menu
}

/// Rebuild the Open Recent submenu from `list`. Item ids are
/// `file.recent.<index>`; duplicate file names get their parent dir appended.
#[cfg(not(target_arch = "wasm32"))]
pub fn rebuild_recent_items(list: &[std::path::PathBuf]) {
    RECENT_SUBMENU.with(|c| {
        let borrowed = c.borrow();
        let Some(sub) = borrowed.as_ref() else { return };
        while sub.remove_at(0).is_some() {}
        if list.is_empty() {
            let none = MenuItem::with_id("file.recent.none", "No Recent Files", false, None);
            let _ = sub.append(&none);
            return;
        }
        let name_of = |p: &std::path::Path| {
            p.file_name().map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        for (i, p) in list.iter().enumerate() {
            let name = name_of(p);
            let duplicated = list.iter().filter(|o| name_of(o) == name).count() > 1;
            let label = match (duplicated, p.parent()) {
                (true, Some(dir)) => format!("{name} — {}", dir.display()),
                _ => name,
            };
            let _ = sub.append(&item(&format!("file.recent.{i}"), &label));
        }
        let _ = sub.append(&PredefinedMenuItem::separator());
        let _ = sub.append(&item("file.recent.clear", "Clear Menu"));
    });
}

/// Best-effort sync of the Auto Update check item with the document state.
#[cfg(not(target_arch = "wasm32"))]
pub fn sync_auto_update_item(on: bool) {
    AUTO_UPDATE_ITEM.with(|c| {
        if let Some(item) = c.borrow().as_ref() {
            item.set_checked(on);
        }
    });
}

/// Web has no native menu handle; the in-app menu bar reads the document
/// state reactively instead.
#[cfg(target_arch = "wasm32")]
pub fn sync_auto_update_item(_on: bool) {}

/// Route a menu event by id. Runs on the UI thread; long flows spawn.
pub fn route(id: &str) {
    match id {
        "file.new" => crate::chrome::file_ops::file_new(),
        "file.open" => crate::chrome::file_ops::file_open(),
        "file.save" => crate::chrome::file_ops::file_save(false),
        "file.saveAs" => crate::chrome::file_ops::file_save(true),
        "file.exportSvg" => crate::export::export_svg(),
        "file.exportPng" => crate::export::export_png(),
        #[cfg(not(target_arch = "wasm32"))]
        "file.recent.clear" => crate::chrome::recent::clear(),
        #[cfg(not(target_arch = "wasm32"))]
        _ if id.starts_with("file.recent.") => {
            if let Some(path) = id
                .strip_prefix("file.recent.")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(crate::chrome::recent::get)
            {
                crate::chrome::file_ops::open_recent(path);
            }
        }
        "edit.undo" => {
            exec(|s| {
                bn_session::ops::edit::undo(s);
                Ok(())
            });
        }
        "edit.redo" => {
            exec(|s| {
                bn_session::ops::edit::redo(s);
                Ok(())
            });
        }
        "edit.selectAll" => crate::state::select_all(),
        "edit.deleteSelection" => crate::state::delete_selection(),
        "network.compile" => {
            exec(|s| {
                bn_session::ops::edit::recompute(s);
                Ok(())
            });
        }
        "network.autoUpdate" => {
            let on = !SESSION.read().doc.auto_update;
            exec(|s| {
                bn_session::ops::edit::set_auto_update(s, on);
                Ok(())
            });
            sync_auto_update_item(on);
        }
        "network.removeFindings" => {
            exec(|s| {
                bn_session::ops::evidence::retract_all_findings(s);
                Ok(())
            });
        }
        "network.rename" => open_dialog(DialogDesc::RenameNetwork),
        "cases.learnCpts" => open_dialog(DialogDesc::LearnCpts),
        "cases.learnStructure" => open_dialog(DialogDesc::StructureLearn),
        "cases.simulate" => open_dialog(DialogDesc::Simulate),
        "tools.sensitivity" => open_dialog(DialogDesc::Sensitivity),
        "tools.arcStrength" => open_dialog(DialogDesc::ArcStrength),
        "tools.solveId" => {
            if let Some(sol) = exec(|s| bn_session::ops::tools::solve_influence_diagram(s)) {
                open_dialog(DialogDesc::IdSolution { text: sol.text });
            }
        }
        "help.about" => open_dialog(DialogDesc::About),
        _ => {
            state::log_message(format!("Unhandled menu id: {id}"));
        }
    }
}
