//! Status bar: network name + modified marker, compile state, P(findings),
//! node/link counts, last update time, message-log toggle.

use dioxus::prelude::*;

use crate::logic::format::exp4;
use crate::state::{SESSION, SHOW_MESSAGES};

#[component]
pub fn StatusBar() -> Element {
    let s = SESSION.read();
    let title = format!("{}{}", s.doc.net.name, if s.doc.modified { " *" } else { "" });
    let conflict = s.bridge.conflict;
    let compiled = s.bridge.compiled;
    let stale = s.bridge.is_dirty();
    let log_p_e = s.bridge.log_p_e;
    let evidence_count = s.doc.evidence.len();
    let n_nodes = s.doc.net.len();
    let n_edges = s.doc.net.edges().len();
    let last_ms = s.bridge.last_compile_ms;
    drop(s);
    let show_messages = *SHOW_MESSAGES.read();

    rsx! {
        div { class: "flex h-7 shrink-0 items-center gap-2 border-t px-2 text-xs",
            span { class: "font-semibold", "{title}" }
            div { class: "h-4 w-px bg-border" }
            if conflict {
                span { class: "font-medium text-red-600", "⚠ conflicting findings" }
            } else if compiled && !stale {
                span { class: "text-green-700", "Compiled" }
            } else {
                span { class: "text-amber-600", "Stale" }
            }
            if let (Some(lpe), true) = (log_p_e, evidence_count > 0) {
                div { class: "h-4 w-px bg-border" }
                {
                    let plural = if evidence_count == 1 { "" } else { "s" };
                    rsx! {
                        span { "P(findings) = {exp4(lpe.exp())} ({evidence_count} finding{plural})" }
                    }
                }
            }
            div { class: "h-4 w-px bg-border" }
            span { "{n_nodes} nodes, {n_edges} links" }
            div { class: "ml-auto flex items-center gap-2",
                if compiled {
                    span { class: "text-muted-foreground", "update {last_ms:.2} ms" }
                }
                button {
                    class: "flex items-center gap-0.5 rounded px-1 hover:bg-accent",
                    onclick: move |_| {
                        let v = *SHOW_MESSAGES.read();
                        *SHOW_MESSAGES.write() = !v;
                    },
                    if show_messages { "▾ messages" } else { "▴ messages" }
                }
            }
        }
    }
}
