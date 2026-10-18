//! Global AI settings. The text role is a cloud LLM (provider, endpoint,
//! key, model, connection test); the structured role is local Laya — just a
//! checkpoint picker with install status, download, and a local test.

use dioxus::prelude::*;

use crate::ai::client;
use crate::ai::laya_local::{self, LayaCheckpoint};
use crate::ai::settings::{self, ProviderKind, RoleConfig};
use crate::state::{DialogDesc, close_dialog, open_dialog};
use crate::ui::{BTN, BTN_PRIMARY, Modal, SELECT, TextInput};

#[component]
pub fn AiSettingsDialog() -> Element {
    let mut all = use_signal(settings::load);
    let mut save_error = use_signal(String::new);

    let on_save = move |_| {
        match settings::save(&all.read()) {
            Ok(()) => {
                crate::state::log_message("AI settings saved.");
                close_dialog();
            }
            Err(e) => save_error.set(format!("Could not save settings: {e}")),
        }
    };

    rsx! {
        Modal {
            title: "AI Settings",
            width: "max-w-xl",
            draggable: false,
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button { class: BTN_PRIMARY, onclick: on_save, "Save" }
            },
            div { class: "space-y-4 text-sm",
                p { class: "text-xs text-muted-foreground",
                    "Text you submit to the text model is sent directly to the configured \
                     provider using your API key (stored in ~/.config/balina/settings.json, \
                     owner-only permissions). The structured model runs locally."
                }
                RoleSection {
                    title: "Text model",
                    hint: "Generative: drafting networks, explanations \
                           (e.g. Claude Sonnet, GPT-5).",
                    cfg: all.read().text.clone(),
                    on_change: move |c| all.write().text = c,
                }
                div { class: "my-3 border-t" }
                StructuredSection {
                    checkpoint: all.read().structured.checkpoint,
                    on_change: move |cp| all.write().structured.checkpoint = cp,
                }
                if !save_error.read().is_empty() {
                    div { class: "text-xs text-red-600", "{save_error}" }
                }
            }
        }
    }
}

/// Local-Laya structured role: checkpoint, install status, download, test.
#[component]
fn StructuredSection(checkpoint: LayaCheckpoint, on_change: EventHandler<LayaCheckpoint>) -> Element {
    let mut test_status = use_signal(String::new);
    let mut test_ok = use_signal(|| false);
    let mut testing = use_signal(|| false);

    let installed = checkpoint.is_installed();

    rsx! {
        div { class: "space-y-2",
            div {
                h3 { class: "font-semibold", "Structured model (local Laya)" }
                p { class: "text-xs text-muted-foreground",
                    "Constrained decisions: state probabilities, CPT suggestions. Runs \
                     on this machine — no API key, nothing leaves your computer."
                }
            }
            div { class: "flex items-center gap-2",
                span { class: "w-24 shrink-0", "Checkpoint" }
                select {
                    class: "{SELECT} flex-1",
                    value: "{checkpoint.id()}",
                    onchange: move |ev| {
                        if let Some(cp) = LayaCheckpoint::from_id(&ev.value()) {
                            test_status.set(String::new());
                            on_change.call(cp);
                        }
                    },
                    for cp in LayaCheckpoint::ALL {
                        option { value: "{cp.id()}", selected: cp == checkpoint, "{cp.label()}" }
                    }
                }
            }
            div { class: "flex items-center gap-2",
                span { class: "w-24 shrink-0", "Status" }
                if installed {
                    span { class: "text-xs text-green-700", "Installed" }
                } else {
                    span { class: "text-xs text-amber-700", "Not installed" }
                }
                span { class: "text-xs text-muted-foreground",
                    "~/.config/balina/models/laya/{checkpoint.id()}"
                }
            }
            div { class: "flex items-center gap-2",
                button {
                    class: BTN,
                    onclick: move |_| {
                        // Replaces this dialog (single slot); settings edits
                        // not yet saved are discarded — checkpoint choice is
                        // re-selectable, so that's acceptable.
                        open_dialog(DialogDesc::LayaDownload { checkpoint });
                    },
                    if installed { "Re-download…" } else { "Download…" }
                }
                button {
                    class: BTN,
                    disabled: *testing.read() || !installed,
                    onclick: move |_| {
                        testing.set(true);
                        test_status.set("Loading model and running a test…".to_string());
                        spawn(async move {
                            let result = laya_local::test_local(checkpoint).await;
                            testing.set(false);
                            match result {
                                Ok(msg) => {
                                    test_ok.set(true);
                                    test_status.set(msg);
                                }
                                Err(msg) => {
                                    test_ok.set(false);
                                    test_status.set(msg);
                                }
                            }
                        });
                    },
                    "Test"
                }
                if !test_status.read().is_empty() {
                    span {
                        class: if *testing.read() {
                            "text-xs text-muted-foreground"
                        } else if *test_ok.read() {
                            "text-xs text-green-700"
                        } else {
                            "text-xs text-red-600"
                        },
                        "{test_status}"
                    }
                }
            }
        }
    }
}

#[component]
fn RoleSection(
    title: String,
    hint: String,
    cfg: RoleConfig,
    on_change: EventHandler<RoleConfig>,
) -> Element {
    let mut test_status = use_signal(String::new);
    let mut test_ok = use_signal(|| false);
    let mut testing = use_signal(|| false);

    let select_cfg = cfg.clone();
    let url_cfg = cfg.clone();
    let key_cfg = cfg.clone();
    let model_cfg = cfg.clone();
    let test_cfg = cfg.clone();

    rsx! {
        div { class: "space-y-2",
            div {
                h3 { class: "font-semibold", "{title}" }
                p { class: "text-xs text-muted-foreground", "{hint}" }
            }
            div { class: "flex items-center gap-2",
                span { class: "w-24 shrink-0", "Provider" }
                select {
                    class: "{SELECT} flex-1",
                    value: "{cfg.provider.id()}",
                    onchange: move |ev| {
                        if let Some(p) = ProviderKind::from_id(&ev.value()) {
                            // New provider: reset endpoint/model to its
                            // defaults, keep the typed key.
                            let mut c = RoleConfig::for_provider(p);
                            c.api_key = select_cfg.api_key.clone();
                            test_status.set(String::new());
                            on_change.call(c);
                        }
                    },
                    for p in ProviderKind::ALL {
                        option { value: "{p.id()}", selected: p == cfg.provider, "{p.label()}" }
                    }
                }
            }
            if cfg.provider.shows_base_url() {
                div { class: "flex items-center gap-2",
                    span { class: "w-24 shrink-0", "URL" }
                    TextInput {
                        value: cfg.base_url.clone(),
                        placeholder: "https://…/chat/completions",
                        oninput: move |v| {
                            on_change.call(RoleConfig { base_url: v, ..url_cfg.clone() });
                        },
                    }
                }
            }
            div { class: "flex items-center gap-2",
                span { class: "w-24 shrink-0", "API key" }
                TextInput {
                    value: cfg.api_key.clone(),
                    password: true,
                    oninput: move |v| {
                        on_change.call(RoleConfig { api_key: v, ..key_cfg.clone() });
                    },
                }
            }
            div { class: "flex items-center gap-2",
                span { class: "w-24 shrink-0", "Model" }
                TextInput {
                    value: cfg.model.clone(),
                    oninput: move |v| {
                        on_change.call(RoleConfig { model: v, ..model_cfg.clone() });
                    },
                }
            }
            div { class: "flex items-center gap-2",
                button {
                    class: BTN,
                    disabled: *testing.read(),
                    onclick: move |_| {
                        let cfg = test_cfg.clone();
                        testing.set(true);
                        test_status.set("Testing…".to_string());
                        spawn(async move {
                            let result = client::test_connection(cfg).await;
                            testing.set(false);
                            match result {
                                Ok(msg) => {
                                    test_ok.set(true);
                                    test_status.set(msg);
                                }
                                Err(msg) => {
                                    test_ok.set(false);
                                    test_status.set(msg);
                                }
                            }
                        });
                    },
                    "Test connection"
                }
                if !test_status.read().is_empty() {
                    span {
                        class: if *testing.read() {
                            "text-xs text-muted-foreground"
                        } else if *test_ok.read() {
                            "text-xs text-green-700"
                        } else {
                            "text-xs text-red-600"
                        },
                        "{test_status}"
                    }
                }
            }
        }
    }
}
