//! Web save dialog: filename + format, then a blob download (browsers have
//! no native save picker we can rely on cross-engine).

use bn_core::io;
use dioxus::prelude::*;

use crate::state::{close_dialog, exec, log_message, SESSION};
use crate::ui::{Modal, TextInput, BTN, BTN_PRIMARY, SELECT};

const FORMATS: &[(io::Format, &str, &str)] = &[
    (io::Format::NativeJson, "Balina (.balina)", "balina"),
    (io::Format::Xmlbif, "XMLBIF (.xmlbif)", "xmlbif"),
    (io::Format::Xdsl, "XDSL / GeNIe (.xdsl)", "xdsl"),
];

#[component]
pub fn SaveAsWebDialog() -> Element {
    let mut name = use_signal(|| SESSION.read().doc.net.name.clone());
    let mut fmt_idx = use_signal(|| 0usize);

    let save = move |_| {
        let (fmt, _, ext) = FORMATS[*fmt_idx.read()];
        let base = name.read().trim().to_string();
        let base = if base.is_empty() { "network".to_string() } else { base };
        let Some((text, warnings)) = exec(|s| bn_session::ops::file::doc_save_str(s, fmt)) else {
            return;
        };
        for w in &warnings {
            log_message(format!("Warning: {w}"));
        }
        let filename = format!("{base}.{ext}");
        crate::platform::download(&filename, "application/octet-stream", text.as_bytes());
        log_message(format!("Saved {filename} (downloaded)."));
        close_dialog();
    };

    rsx! {
        Modal {
            title: "Save network",
            width: "max-w-sm",
            draggable: false, // configuration dialog
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button { class: BTN_PRIMARY, onclick: save, "Save" }
            },
            div { class: "space-y-3 text-sm",
                div { class: "flex items-center gap-2",
                    label { class: "w-24", "File name:" }
                    TextInput {
                        value: name.read().clone(),
                        autofocus: true,
                        class: "flex-1".to_string(),
                        oninput: move |v| name.set(v),
                    }
                }
                div { class: "flex items-center gap-2",
                    label { class: "w-24", "Format:" }
                    select {
                        class: SELECT,
                        onchange: move |ev| {
                            if let Ok(i) = ev.value().parse::<usize>() {
                                fmt_idx.set(i.min(FORMATS.len() - 1));
                            }
                        },
                        for (i, (_, label, _)) in FORMATS.iter().enumerate() {
                            option { value: "{i}", selected: *fmt_idx.read() == i, "{label}" }
                        }
                    }
                }
                p { class: "text-xs text-muted-foreground",
                    "The file is downloaded by the browser."
                }
            }
        }
    }
}
