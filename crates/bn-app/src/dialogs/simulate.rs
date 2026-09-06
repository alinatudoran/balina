//! Simulate cases from the current network into a CSV file. The generation
//! runs off the UI thread on a network clone (no session borrow held).

use dioxus::prelude::*;

use crate::state::{close_dialog, log_message, SESSION};
use crate::ui::{Modal, NumberInput, BTN, BTN_PRIMARY};

#[component]
pub fn SimulateDialog() -> Element {
    let mut n = use_signal(|| 1000usize);
    let mut missing_pct = use_signal(|| 0.0f64);
    let mut running = use_signal(|| false);

    let generate = move |_| {
        spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_file_name("cases.csv")
                .save_file()
                .await;
            let Some(fh) = picked else { return };
            let path = fh.path().to_path_buf();
            running.set(true);
            let net = SESSION.read().doc.net.clone();
            let (count, pct) = (*n.read(), *missing_pct.read());
            let result = tokio::task::spawn_blocking(move || {
                bn_session::ops::learn::simulate_cases_to_file(&net, path, count, pct)
            })
            .await;
            running.set(false);
            match result {
                Ok(Ok(msg)) => {
                    log_message(msg);
                    close_dialog();
                }
                Ok(Err(e)) => log_message(format!("Simulation failed: {e}")),
                Err(e) => log_message(format!("Simulation crashed: {e}")),
            }
        });
    };

    rsx! {
        Modal {
            title: "Simulate cases",
            width: "max-w-sm",
            draggable: false, // configuration dialog
            on_close: move |_| close_dialog(),
            footer: rsx! {
                button { class: BTN, onclick: move |_| close_dialog(), "Cancel" }
                button {
                    class: BTN_PRIMARY,
                    disabled: *running.read(),
                    onclick: generate,
                    "Generate…"
                }
            },
            div { class: "space-y-3 text-sm",
                div { class: "flex items-center gap-2",
                    label { class: "w-40", "Number of cases:" }
                    NumberInput {
                        class: "h-8 w-28".to_string(),
                        min: "1".to_string(),
                        max: "1000000".to_string(),
                        value: n.read().to_string(),
                        onchange: move |v: String| {
                            if let Ok(v) = v.parse::<usize>() {
                                n.set(v.clamp(1, 1_000_000));
                            }
                        },
                    }
                }
                div { class: "flex items-center gap-2",
                    label { class: "w-40", "Missing values (%):" }
                    NumberInput {
                        class: "h-8 w-28".to_string(),
                        min: "0".to_string(),
                        max: "90".to_string(),
                        value: missing_pct.read().to_string(),
                        onchange: move |v: String| {
                            if let Ok(v) = v.parse::<f64>() {
                                missing_pct.set(v.clamp(0.0, 90.0));
                            }
                        },
                    }
                }
            }
        }
    }
}
