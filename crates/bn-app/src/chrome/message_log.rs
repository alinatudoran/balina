//! Collapsible message log, stick-to-bottom, capped at 300 entries (the cap
//! lives in `state::log_message`).

use dioxus::prelude::*;

use crate::state::{MESSAGES, SHOW_MESSAGES};

#[component]
pub fn MessageLog() -> Element {
    let mut end_el: Signal<Option<std::rc::Rc<MountedData>>> = use_signal(|| None);

    // Stick to bottom whenever messages change while visible.
    use_effect(move || {
        let _len = MESSAGES.read().len();
        let _shown = *SHOW_MESSAGES.read();
        if let Some(el) = end_el.read().clone() {
            spawn(async move {
                let _ = el.scroll_to(ScrollBehavior::Instant).await;
            });
        }
    });

    if !*SHOW_MESSAGES.read() {
        return rsx! {};
    }
    rsx! {
        div { class: "h-24 shrink-0 overflow-y-auto border-t bg-muted/30 px-2 py-1 select-text",
            for (i, m) in MESSAGES.read().iter().enumerate() {
                div { key: "{i}", class: "whitespace-pre-wrap font-mono text-[11px] leading-4", "{m}" }
            }
            div { onmounted: move |ev| end_el.set(Some(ev.data())) }
        }
    }
}
