//! Web file flows: open through the browser file input (rfd wasm), save and
//! export as blob downloads. Same public surface as the desktop `file_ops`.

use bn_core::io;
use bn_session::ops;
use dioxus::prelude::*;

use crate::state::{exec, log_message, open_dialog, DialogDesc, SESSION};

const OPEN_FILTERS: &[(&str, &[&str])] = &[
    ("All networks", &["balina", "json", "xml", "bif", "xmlbif", "xdsl"]),
    ("Balina", &["balina", "json"]),
    ("XMLBIF", &["xml", "bif", "xmlbif"]),
    ("XDSL (GeNIe)", &["xdsl"]),
];

/// True to proceed (document unmodified, or the user confirmed discarding).
async fn confirm_discard(action: &str) -> bool {
    if !SESSION.read().doc.modified {
        return true;
    }
    crate::platform::confirm(
        "Unsaved changes",
        &format!("The network has unsaved changes. {action} anyway?"),
    )
    .await
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
        log_message("New network.");
    });
}

pub fn file_open() {
    spawn(async move {
        if !confirm_discard("Discard them and open another file").await {
            return;
        }
        let mut d = rfd::AsyncFileDialog::new();
        for (name, exts) in OPEN_FILTERS {
            d = d.add_filter(*name, exts);
        }
        let Some(fh) = d.pick_file().await else { return };
        let name = fh.file_name();
        let fmt = match io::Format::from_path(std::path::Path::new(&name)) {
            Ok(f) => f,
            Err(e) => {
                log_message(format!("Open failed: {e}"));
                return;
            }
        };
        let text = match String::from_utf8(fh.read().await) {
            Ok(t) => t,
            Err(e) => {
                log_message(format!("Open failed: {e}"));
                return;
            }
        };
        if exec(|s| ops::file::doc_open_str(s, &text, fmt)).is_some() {
            crate::state::clear_selection();
            *crate::canvas::controller::FIT_REQUEST.write() += 1;
            let (name, n) = {
                let s = SESSION.read();
                (s.doc.net.name.clone(), s.doc.net.len())
            };
            log_message(format!("Loaded `{name}` ({n} nodes)."));
        }
    });
}

/// Save and Save As are the same on the web: pick a name + format, download.
pub fn file_save(_save_as: bool) {
    open_dialog(DialogDesc::SaveAsWeb);
}
