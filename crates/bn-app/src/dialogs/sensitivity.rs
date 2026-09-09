//! Sensitivity to findings: pick a target, rank the other nodes by mutual
//! information. Modeless so findings can be changed while it is open.

use bn_core::model::{NodeId, NodeKind};
use bn_session::views::SensRowView;
use dioxus::prelude::*;

use crate::state::{close_dialog, exec, log_message, SESSION};
use crate::ui::{Modeless, BTN_PRIMARY, BTN_SM, SELECT};

#[component]
pub fn SensitivityDialog() -> Element {
    let mut target: Signal<Option<NodeId>> = use_signal(|| None);
    let mut rows: Signal<Vec<SensRowView>> = use_signal(Vec::new);

    let candidates: Vec<(NodeId, String)> = {
        let s = SESSION.read();
        s.doc
            .net
            .nodes()
            .filter(|(_, n)| n.kind != NodeKind::Utility)
            .map(|(id, n)| (id, n.name.clone()))
            .collect()
    };
    let target_valid =
        target.read().is_some_and(|t| candidates.iter().any(|(id, _)| *id == t));

    // Rows referencing deleted nodes are dropped lazily.
    let live_rows: Vec<SensRowView> = {
        let s = SESSION.read();
        rows.read().iter().filter(|r| s.doc.net.contains(r.node)).cloned().collect()
    };
    let max_mi = live_rows.first().map(|r| r.mutual_info).unwrap_or(0.0).max(1e-12);

    let cand_for_select = candidates.clone();
    let run = move |_| {
        let Some(t) = *target.read() else { return };
        if let Some(r) = exec(|s| bn_session::ops::tools::run_sensitivity(s, t)) {
            rows.set(r);
        }
    };

    // CSV of the rows still valid at click time (same staleness filter as
    // the table); the dialog stays open so the user can keep exploring.
    let export_csv = move || -> Option<String> {
        let (target_name, live) = {
            let s = SESSION.read();
            let target_name = target
                .read()
                .filter(|&t| s.doc.net.contains(t))
                .map(|t| s.doc.net.node(t).name.clone())
                .unwrap_or_else(|| "(deleted)".into());
            let live: Vec<SensRowView> =
                rows.read().iter().filter(|r| s.doc.net.contains(r.node)).cloned().collect();
            (target_name, live)
        };
        match bn_session::ops::tools::sensitivity_csv(&target_name, &live) {
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
                .set_file_name("sensitivity.csv")
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
        crate::platform::download("sensitivity.csv", "text/csv", csv.as_bytes());
        log_message("Exported sensitivity.csv (downloaded).".to_string());
    };

    rsx! {
        Modeless {
            title: "Sensitivity to findings",
            width: "max-w-xl",
            on_close: move |_| close_dialog(),
            div { class: "flex items-center gap-2 text-sm",
                span { "Target node:" }
                select {
                    class: "{SELECT} w-44",
                    onchange: move |ev| {
                        target.set(
                            ev.value()
                                .parse::<usize>()
                                .ok()
                                .and_then(|i| cand_for_select.get(i).map(|(id, _)| *id)),
                        );
                    },
                    option { value: "none", selected: target.read().is_none(), "—" }
                    for (i, (id, name)) in candidates.iter().enumerate() {
                        option {
                            value: "{i}",
                            selected: *target.read() == Some(*id),
                            "{name}"
                        }
                    }
                }
                button {
                    class: "{BTN_PRIMARY} h-7 text-xs",
                    disabled: !target_valid,
                    onclick: run,
                    "Run"
                }
                button {
                    class: BTN_SM,
                    disabled: live_rows.is_empty(),
                    onclick: save,
                    "Save CSV…"
                }
            }
            if !live_rows.is_empty() {
                div { class: "max-h-80 overflow-auto rounded border",
                    table { class: "w-full text-xs",
                        thead { class: "sticky top-0 bg-muted",
                            tr {
                                th { class: "px-2 py-1.5 text-left font-semibold", "Node" }
                                th { class: "px-2 py-1.5 text-left font-semibold",
                                    "Mutual info (bits)"
                                }
                                th { class: "px-2 py-1.5 text-left font-semibold",
                                    "Entropy red. %"
                                }
                                th { class: "px-2 py-1.5 text-left font-semibold",
                                    "Variance red."
                                }
                            }
                        }
                        tbody {
                            for r in live_rows.iter() {
                                tr { class: "border-t odd:bg-muted/30",
                                    td { class: "px-2 py-1", "{r.name}" }
                                    td { class: "px-2 py-1",
                                        div { class: "flex items-center gap-1.5",
                                            div { class: "relative h-3 w-20 rounded-sm bg-neutral-200",
                                                div {
                                                    class: "absolute inset-y-0 left-0 rounded-sm",
                                                    style: "width: {(r.mutual_info / max_mi * 100.0).min(100.0):.1}%; \
                                                            background: rgb(220,150,60);",
                                                }
                                            }
                                            span { class: "font-mono", "{r.mutual_info:.5}" }
                                        }
                                    }
                                    td { class: "px-2 py-1 font-mono",
                                        "{r.entropy_reduction_pct:.2}"
                                    }
                                    td { class: "px-2 py-1 font-mono",
                                        match r.variance_reduction {
                                            Some(v) => rsx! { "{v:.5}" },
                                            None => rsx! { "—" },
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
