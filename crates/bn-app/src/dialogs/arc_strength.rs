//! Arc strength table: for each arc in the network, the mutual information
//! I(parent; child | evidence) from compiled beliefs, sorted descending.

use bn_session::views::ArcStrengthRow;
use dioxus::prelude::*;

use crate::state::{close_dialog, exec, log_message, SESSION};
use crate::ui::{Modeless, BTN_PRIMARY, BTN_SM};

#[component]
pub fn ArcStrengthDialog() -> Element {
    let mut rows: Signal<Vec<ArcStrengthRow>> = use_signal(Vec::new);
    let mut error: Signal<Option<String>> = use_signal(|| None);

    let arc_count = SESSION.read().doc.net.edges().len();
    let can_run = arc_count > 0;

    let live_rows: Vec<ArcStrengthRow> = {
        let s = SESSION.read();
        rows.read()
            .iter()
            .filter(|r| s.doc.net.contains(r.parent) && s.doc.net.contains(r.child))
            .cloned()
            .collect()
    };
    let max_mi = live_rows.first().map(|r| r.mutual_info).unwrap_or(0.0).max(1e-12);

    let run = move |_| {
        match exec(|s| bn_session::ops::tools::run_arc_strengths(s)) {
            Some(r) => {
                rows.set(r);
                error.set(None);
            }
            None => error.set(Some("Network must be compiled first.".into())),
        }
    };

    // CSV of the rows still valid at click time (same staleness filter as
    // the table); the dialog stays open so the user can keep exploring.
    let export_csv = move || -> Option<String> {
        let live: Vec<ArcStrengthRow> = {
            let s = SESSION.read();
            rows.read()
                .iter()
                .filter(|r| s.doc.net.contains(r.parent) && s.doc.net.contains(r.child))
                .cloned()
                .collect()
        };
        match bn_session::ops::tools::arc_strengths_csv(&live) {
            Ok(csv) => Some(csv),
            Err(e) => {
                log_message(format!("CSV export failed: {e}"));
                None
            }
        }
    };

    #[cfg(not(target_arch = "wasm32"))]
    let save = move |_| {
        let Some(csv) = export_csv() else { return };
        spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_file_name("arc-strengths.csv")
                .save_file()
                .await;
            let Some(fh) = picked else { return };
            let path = fh.path().to_path_buf();
            match std::fs::write(&path, csv) {
                Ok(()) => log_message(format!("Exported {}.", path.display())),
                Err(e) => log_message(format!("CSV export failed: {e}")),
            }
        });
    };

    #[cfg(target_arch = "wasm32")]
    let save = move |_| {
        let Some(csv) = export_csv() else { return };
        crate::platform::download("arc-strengths.csv", "text/csv", csv.as_bytes());
        log_message("Exported arc-strengths.csv (downloaded).".to_string());
    };

    rsx! {
        Modeless {
            title: "Arc strengths",
            width: "max-w-xl",
            on_close: move |_| close_dialog(),
            div { class: "flex items-center gap-2 text-sm",
                span { class: "text-muted-foreground", "{arc_count} arc(s) in network" }
                button {
                    class: "{BTN_PRIMARY} h-7 text-xs",
                    disabled: !can_run,
                    onclick: run,
                    "Compute"
                }
                button {
                    class: BTN_SM,
                    disabled: live_rows.is_empty(),
                    onclick: save,
                    "Save CSV…"
                }
            }
            if let Some(ref e) = *error.read() {
                div { class: "text-xs text-red-600", "{e}" }
            }
            if !live_rows.is_empty() {
                div { class: "max-h-80 overflow-auto rounded border",
                    table { class: "w-full text-xs",
                        thead { class: "sticky top-0 bg-muted",
                            tr {
                                th { class: "px-2 py-1.5 text-left font-semibold", "Parent" }
                                th { class: "px-2 py-1.5 text-left font-semibold", "Child" }
                                th { class: "px-2 py-1.5 text-left font-semibold",
                                    "Strength (bits)"
                                }
                            }
                        }
                        tbody {
                            for r in live_rows.iter() {
                                tr { class: "border-t odd:bg-muted/30",
                                    td { class: "px-2 py-1 font-mono", "{r.parent_name}" }
                                    td { class: "px-2 py-1 font-mono", "→ {r.child_name}" }
                                    td { class: "px-2 py-1",
                                        div { class: "flex items-center gap-1.5",
                                            div {
                                                class: "relative h-3 w-24 rounded-sm bg-neutral-200",
                                                div {
                                                    class: "absolute inset-y-0 left-0 rounded-sm bg-blue-500",
                                                    style: "width: {(r.mutual_info / max_mi * 100.0).min(100.0):.1}%;",
                                                }
                                            }
                                            span { class: "font-mono", "{r.mutual_info:.5}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
