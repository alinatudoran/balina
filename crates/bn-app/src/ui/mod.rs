//! Minimal UI kit, modeled on Dioxus/UI (rust-ui/dioxus-ui, MIT) / shadcn
//! styling but hand-rolled: plain class strings, no tw_merge, and text
//! inputs wire the global TYPING flag (the not-while-typing hotkey guard).

use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::state::{TOAST, TYPING};

pub const BTN: &str = "inline-flex h-8 cursor-default items-center justify-center gap-1 \
    rounded-md border bg-background px-3 text-sm shadow-xs hover:bg-accent \
    disabled:pointer-events-none disabled:opacity-50";
pub const BTN_PRIMARY: &str = "inline-flex h-8 cursor-default items-center justify-center gap-1 \
    rounded-md bg-primary px-3 text-sm text-primary-foreground shadow-xs hover:bg-primary/90 \
    disabled:pointer-events-none disabled:opacity-50";
pub const BTN_SM: &str = "inline-flex h-7 cursor-default items-center justify-center gap-1 \
    rounded-md border bg-background px-2 text-xs shadow-xs hover:bg-accent \
    disabled:pointer-events-none disabled:opacity-50";
pub const BTN_ICON: &str = "inline-flex size-6 cursor-default items-center justify-center \
    rounded hover:bg-accent disabled:pointer-events-none disabled:opacity-40";
pub const INPUT: &str = "h-8 w-full rounded-md border bg-background px-2 text-sm shadow-xs \
    outline-none focus:ring-2 focus:ring-ring/50";
pub const SELECT: &str = "h-8 rounded-md border bg-background px-2 text-sm shadow-xs";

/// Title-bar drag state shared by Modal/Modeless: a pixel offset applied on
/// top of the CSS centering transform, plus the in-flight drag anchor
/// (start client position, offset at drag start).
#[derive(Clone, Copy, PartialEq)]
struct PanelDrag {
    offset: (f64, f64),
    drag: Option<((f64, f64), (f64, f64))>,
}

impl PanelDrag {
    fn new() -> Self {
        PanelDrag { offset: (0.0, 0.0), drag: None }
    }
}

/// Draggable title bar. Mouse events during the drag are handled by
/// [`DragCapture`], which the host panel renders as a sibling.
#[component]
fn PanelTitleBar(
    title: String,
    on_close: EventHandler<()>,
    draggable: bool,
    state: Signal<PanelDrag>,
) -> Element {
    let mut state = state;
    let bar_class = if draggable {
        "flex cursor-move select-none items-center justify-between"
    } else {
        "flex items-center justify-between"
    };
    rsx! {
        div {
            class: bar_class,
            onmousedown: move |ev| {
                if !draggable || ev.data().trigger_button() != Some(MouseButton::Primary) {
                    return;
                }
                let c = ev.data().client_coordinates();
                let mut s = state.write();
                s.drag = Some(((c.x, c.y), s.offset));
            },
            h2 { class: "text-sm font-semibold", "{title}" }
            button {
                class: "rounded-xs opacity-70 hover:opacity-100",
                onmousedown: move |ev| ev.stop_propagation(),
                onclick: move |_| on_close.call(()),
                "✕"
            }
        }
    }
}

/// Full-screen capture layer while a panel drag is in flight (mouse events
/// keep arriving even during fast drags; a release outside the window shows
/// up as a move with no held buttons). MUST be a SIBLING of the panel, not
/// a child: the panel's `transform` makes it the containing block for fixed
/// descendants, so inside the panel `inset-0` only covers the panel itself
/// and a fast drag escapes the layer (the panel stops following).
#[component]
fn DragCapture(state: Signal<PanelDrag>) -> Element {
    let mut state = state;
    rsx! {
        if state.read().drag.is_some() {
            div {
                class: "fixed inset-0 z-[70] cursor-move",
                onmousemove: move |ev| {
                    let cur = *state.peek(); // copy out — the guard must not
                    let Some((start, off0)) = cur.drag else { return }; // outlive the writes below
                    let c = ev.data().client_coordinates();
                    if ev.data().held_buttons().is_empty() {
                        state.write().drag = None;
                        return;
                    }
                    state.write().offset = (off0.0 + c.x - start.0, off0.1 + c.y - start.1);
                },
                onmouseup: move |_| state.write().drag = None,
            }
        }
    }
}

/// Modal dialog: overlay + centered panel, draggable by the title bar
/// unless `draggable: false` (configuration dialogs). Escape is handled by
/// the global hotkeys (closes `DIALOG`).
#[component]
pub fn Modal(
    title: String,
    on_close: EventHandler<()>,
    #[props(optional)] footer: Option<Element>,
    #[props(default = "max-w-md".to_string())] width: String,
    #[props(default = true)] draggable: bool,
    children: Element,
) -> Element {
    let state = use_signal(PanelDrag::new);
    let (ox, oy) = state.read().offset;
    rsx! {
        div {
            class: "fixed inset-0 z-50 bg-black/50",
            onclick: move |_| on_close.call(()),
        }
        div {
            class: "fixed top-1/2 left-1/2 z-50 grid w-full {width} gap-3 rounded-lg border \
                    bg-background p-4 shadow-lg",
            style: "transform: translate(calc(-50% + {ox:.1}px), calc(-50% + {oy:.1}px));",
            onclick: move |ev| ev.stop_propagation(),
            PanelTitleBar { title, on_close, draggable, state }
            {children}
            if let Some(f) = footer {
                div { class: "flex justify-end gap-2", {f} }
            }
        }
        DragCapture { state }
    }
}

/// Non-modal floating panel (no overlay, no focus trap): the canvas stays
/// interactive while it is open — parity with the egui app's modeless
/// windows (CPT editor, sensitivity). Draggable by the title bar.
#[component]
pub fn Modeless(
    title: String,
    on_close: EventHandler<()>,
    #[props(optional)] footer: Option<Element>,
    #[props(default = "max-w-lg".to_string())] width: String,
    children: Element,
) -> Element {
    let state = use_signal(PanelDrag::new);
    let (ox, oy) = state.read().offset;
    rsx! {
        div {
            class: "fixed top-16 left-1/2 z-50 grid w-full {width} gap-3 rounded-lg border \
                    bg-background p-4 shadow-lg",
            style: "transform: translate(calc(-50% + {ox:.1}px), {oy:.1}px);",
            onmousedown: move |ev| ev.stop_propagation(),
            PanelTitleBar { title, on_close, draggable: true, state }
            {children}
            if let Some(f) = footer {
                div { class: "flex justify-end gap-2", {f} }
            }
        }
        DragCapture { state }
    }
}

/// How long a toast stays up before [`ToastHost`] dismisses it.
const TOAST_DISMISS_MS: u64 = 3000;

/// Success toast pinned to the top center, above modals. Purely
/// presentational — mount through [`ToastHost`], which owns dismissal.
#[component]
pub fn Toast(message: String) -> Element {
    rsx! {
        div { class: "pointer-events-none fixed top-3 left-1/2 z-[80] -translate-x-1/2",
            div {
                class: "flex items-center gap-2 rounded-md border border-green-200 bg-green-50 \
                        px-4 py-2 text-sm text-green-800 shadow-lg",
                span { class: "font-semibold", "✓" }
                "{message}"
            }
        }
    }
}

/// Renders the single-slot global toast ([`crate::state::TOAST`]) and
/// auto-dismisses it. The sequence number keeps a timer from an
/// already-replaced toast from cutting the newer one short.
#[component]
pub fn ToastHost() -> Element {
    use_effect(|| {
        let Some(seq) = TOAST.read().as_ref().map(|(s, _)| *s) else {
            return;
        };
        spawn(async move {
            crate::platform::sleep_ms(TOAST_DISMISS_MS).await;
            // Copy the check out — never write while a peek guard is alive.
            let current = TOAST.peek().as_ref().is_some_and(|(s, _)| *s == seq);
            if current {
                *TOAST.write() = None;
            }
        });
    });
    let message = TOAST.read().as_ref().map(|(_, m)| m.clone());
    rsx! {
        if let Some(message) = message {
            Toast { message }
        }
    }
}

/// Text input that maintains the TYPING flag (hotkey guard). ALL free-text
/// entry must go through this or `TextArea` — a raw `input {}` silently
/// breaks the guard.
#[component]
pub fn TextInput(
    value: String,
    oninput: EventHandler<String>,
    #[props(default = String::new())] class: String,
    #[props(optional)] placeholder: Option<String>,
    #[props(default = false)] autofocus: bool,
    #[props(default = false)] password: bool,
    #[props(optional)] id: Option<String>,
) -> Element {
    rsx! {
        input {
            id,
            class: "{INPUT} {class}",
            r#type: if password { "password" } else { "text" },
            value: "{value}",
            placeholder,
            autofocus,
            onfocus: move |_| *TYPING.write() = true,
            onblur: move |_| *TYPING.write() = false,
            oninput: move |ev| oninput.call(ev.value()),
        }
    }
}

/// Numeric input that maintains the TYPING flag — same rule as
/// [`TextInput`]: a raw `input {{ r#type: "number" }}` breaks the guard and
/// Delete/Backspace get swallowed by the delete-selection hotkey. The value
/// is committed on change (blur / stepper); call sites parse and clamp.
#[component]
pub fn NumberInput(
    value: String,
    onchange: EventHandler<String>,
    #[props(optional)] min: Option<String>,
    #[props(optional)] max: Option<String>,
    #[props(optional)] step: Option<String>,
    #[props(default = "h-7 w-16".to_string())] class: String,
) -> Element {
    rsx! {
        input {
            class: "rounded border bg-background px-1 text-xs outline-none \
                    focus:ring-2 focus:ring-ring/50 {class}",
            r#type: "number",
            min,
            max,
            step,
            value: "{value}",
            onfocus: move |_| *TYPING.write() = true,
            onblur: move |_| *TYPING.write() = false,
            onchange: move |ev| onchange.call(ev.value()),
        }
    }
}

/// Multi-line input maintaining the TYPING flag. `unstyled` drops the boxed
/// look (border/background/shadow) for hosts that provide their own — merely
/// appending overriding classes does not work (equal CSS specificity, the
/// stylesheet order wins). `onblur` fires AFTER the TYPING flag clears.
#[component]
pub fn TextArea(
    value: String,
    oninput: EventHandler<String>,
    #[props(default = 2)] rows: i64,
    #[props(default = String::new())] class: String,
    #[props(default = String::new())] style: String,
    #[props(default = false)] unstyled: bool,
    #[props(default = false)] autofocus: bool,
    #[props(optional)] onblur: Option<EventHandler<()>>,
    #[props(optional)] onkeydown: Option<EventHandler<KeyboardEvent>>,
) -> Element {
    let base = if unstyled {
        "outline-none"
    } else {
        "w-full rounded-md border bg-background px-2 py-1 text-sm shadow-xs \
         outline-none focus:ring-2 focus:ring-ring/50"
    };
    rsx! {
        textarea {
            class: "{base} {class}",
            style,
            rows: "{rows}",
            value: "{value}",
            autofocus,
            // The `autofocus` attribute is ignored for dynamically inserted
            // elements (WKWebView) — focus explicitly on mount instead.
            onmounted: move |ev| {
                if autofocus {
                    spawn(async move {
                        let _ = ev.data().set_focus(true).await;
                    });
                }
            },
            onfocus: move |_| *TYPING.write() = true,
            onblur: move |_| {
                *TYPING.write() = false;
                if let Some(cb) = &onblur {
                    cb.call(());
                }
            },
            onkeydown: move |ev| {
                if let Some(cb) = &onkeydown {
                    cb.call(ev);
                }
            },
            oninput: move |ev| oninput.call(ev.value()),
        }
    }
}
