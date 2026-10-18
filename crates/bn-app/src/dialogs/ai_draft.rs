//! Draft-network-from-text dialog: paste a description, the text model
//! proposes a network, validation gates it, and the accepted draft opens as
//! a new unsaved tab (keeping or saving the tab is the user's acceptance).

use dioxus::prelude::*;

use crate::ai::{draft, settings};
use crate::state::{close_dialog, exec, log_message};
use crate::ui::{Modal, TextArea, BTN, BTN_PRIMARY};

#[component]
pub fn AiDraftDialog() -> Element {
    let mut text = use_signal(String::new);
    let mut running = use_signal(|| false);
    let mut error = use_signal(String::new);

    let can_draft = !*running.read() && !text.read().trim().is_empty();
    let on_draft = move |_| {
        let description = text.read().trim().to_string();
        let cfg = settings::load().text;
        running.set(true);
        error.set(String::new());
        spawn(async move {
            let result = draft::draft_network(cfg, description).await;
            running.set(false);
            match result {
                Ok(net) => {
                    let name = net.name.clone();
                    exec(|s| Ok(bn_session::ops::tab::add_tab_with_network(s, net)));
                    // Same post-add gesture as `tab_bar::add_new_tab`: frame
                    // the fresh network in the viewport.
                    *crate::canvas::controller::FIT_REQUEST.write() += 1;
                    log_message(format!(
                        "AI draft `{name}` opened as a new tab — review before saving."
                    ));
                    close_dialog();
                }
                Err(e) => error.set(e),
            }
        });
    };

    rsx! {
        Modal {
            title: "Draft Network from Text",
            width: "max-w-2xl",
            draggable: false,
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button {
                    class: BTN_PRIMARY,
                    disabled: !can_draft,
                    onclick: on_draft,
                    if *running.read() { "Drafting…" } else { "Draft" }
                }
            },
            div { class: "space-y-2 text-sm",
                p { class: "text-xs text-muted-foreground",
                    "Paste a description or document. The text model proposes nodes, \
                     states, and links; CPTs start uniform. The draft is validated \
                     (cycles, states, names) and opens as a new unsaved tab for review. \
                     The text is sent to the configured provider."
                }
                TextArea {
                    rows: 12,
                    value: text.read().clone(),
                    autofocus: true,
                    oninput: move |v| text.set(v),
                }
                if *running.read() {
                    div { class: "text-xs text-muted-foreground", "Asking the text model…" }
                }
                if !error.read().is_empty() {
                    div { class: "max-h-32 overflow-auto text-xs whitespace-pre-wrap text-red-600",
                        "{error}"
                    }
                }
            }
        }
    }
}
