//! File open/save flows: rfd native pickers in-process, then plain
//! path-taking session ops.

use std::path::PathBuf;

use bn_session::ops;
use dioxus::prelude::*;

use crate::state::{exec, log_message, SESSION};

const OPEN_FILTERS: &[(&str, &[&str])] = &[
    ("All networks", &["balina", "json", "xml", "bif", "xmlbif", "xdsl"]),
    ("Balina", &["balina", "json"]),
    ("XMLBIF", &["xml", "bif", "xmlbif"]),
    ("XDSL (GeNIe)", &["xdsl"]),
];

const SAVE_FILTERS: &[(&str, &[&str])] = &[
    ("Balina", &["balina"]),
    ("XMLBIF", &["xmlbif", "xml"]),
    ("XDSL (GeNIe)", &["xdsl"]),
];

fn with_filters(mut d: rfd::AsyncFileDialog, filters: &[(&str, &[&str])]) -> rfd::AsyncFileDialog {
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    d
}

/// True to proceed (document unmodified, or the user confirmed discarding).
async fn confirm_discard(action: &str) -> bool {
    if !SESSION.read().doc.modified {
        return true;
    }
    let choice = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("Unsaved changes")
        .set_description(format!("The network has unsaved changes. {action} anyway?"))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show()
        .await;
    matches!(choice, rfd::MessageDialogResult::Ok)
}

pub fn file_new() {
    spawn(async move {
        if !confirm_discard("Discard them and start a new network").await {
            return;
        }
        exec(|s| {
            ops::file::doc_new(s);
            Ok(())
        });
        crate::state::clear_selection();
        crate::chrome::menu::sync_auto_update_item(true);
        log_message("New network.");
    });
}

pub fn file_open() {
    spawn(async move {
        if !confirm_discard("Discard them and open another file").await {
            return;
        }
        let picked = with_filters(rfd::AsyncFileDialog::new(), OPEN_FILTERS).pick_file().await;
        let Some(fh) = picked else { return };
        open_path(fh.path().to_path_buf());
    });
}

/// Open a recent file: same discard confirmation as Open…, no picker.
pub fn open_recent(path: PathBuf) {
    spawn(async move {
        if !confirm_discard("Discard them and open another file").await {
            return;
        }
        if !path.is_file() {
            log_message(format!("File not found: {}", path.display()));
            return;
        }
        open_path(path);
    });
}

/// Open a specific path (initial CLI file, or a picked one).
pub fn open_path(path: PathBuf) {
    let recent = path.clone();
    if exec(|s| ops::file::doc_open(s, path)).is_some() {
        crate::chrome::recent::add(&recent);
        crate::state::clear_selection();
        *crate::canvas::controller::FIT_REQUEST.write() += 1;
        let auto = SESSION.read().doc.auto_update;
        crate::chrome::menu::sync_auto_update_item(auto);
        let (name, n) = {
            let s = SESSION.read();
            (s.doc.net.name.clone(), s.doc.net.len())
        };
        log_message(format!("Loaded `{name}` ({n} nodes)."));
    }
}

pub fn file_save(save_as: bool) {
    spawn(async move {
        let (has_path, name) = {
            let s = SESSION.read();
            (s.doc.path.is_some(), s.doc.net.name.clone())
        };
        let mut path: Option<PathBuf> = None;
        if save_as || !has_path {
            let picked = with_filters(rfd::AsyncFileDialog::new(), SAVE_FILTERS)
                .set_file_name(format!("{name}.balina"))
                .save_file()
                .await;
            let Some(fh) = picked else { return };
            path = Some(fh.path().to_path_buf());
        }
        if let Some(result) = exec(|s| ops::file::doc_save(s, path)) {
            for w in &result.warnings {
                log_message(format!("Warning: {w}"));
            }
            crate::chrome::recent::add(&result.path);
            log_message(format!("Saved {}.", result.path.display()));
        }
    });
}
