//! Frontend hotkeys with the not-while-typing guard: Cmd+Z / Cmd+Shift+Z,
//! Cmd+A, Delete/Backspace, Cmd+= / Cmd+- / Cmd+0 (zoom), Escape. Native
//! menu accelerators own only Cmd+N/O/S, Cmd+Shift+S and F5 (see
//! chrome::menu).

use dioxus::events::KeyboardEvent;
use dioxus::html::input_data::keyboard_types::{Key, Modifiers};
use dioxus::prelude::*;

use crate::canvas::controller;
use crate::state::{self, exec, CONTEXT_MENU, DIALOG, TYPING};

/// Root `onkeydown` handler (the root div has `tabindex: 0`).
pub fn handle_keydown(ev: KeyboardEvent) {
    let data = ev.data();
    let key = data.key();

    // Escape always works: close menu/dialog, cancel an in-flight gesture.
    if key == Key::Escape {
        if CONTEXT_MENU.read().is_some() {
            *CONTEXT_MENU.write() = None;
            return;
        }
        if DIALOG.read().is_some() {
            *DIALOG.write() = None;
            return;
        }
        controller::cancel_gesture();
        return;
    }

    // Edit hotkeys are ignored while a text input has focus.
    if *TYPING.read() {
        return;
    }

    let mods = data.modifiers();
    let cmd = mods.contains(Modifiers::META) || mods.contains(Modifiers::CONTROL);
    let shift = mods.contains(Modifiers::SHIFT);

    match key {
        Key::Character(ref k) if cmd && k.eq_ignore_ascii_case("z") => {
            ev.prevent_default();
            if shift {
                exec(|s| {
                    bn_session::ops::edit::redo(s);
                    Ok(())
                });
            } else {
                exec(|s| {
                    bn_session::ops::edit::undo(s);
                    Ok(())
                });
            }
        }
        Key::Character(ref k) if cmd && k.eq_ignore_ascii_case("a") => {
            ev.prevent_default();
            state::select_all();
        }
        Key::Character(ref k) if cmd && (k == "=" || k == "+") => {
            ev.prevent_default();
            controller::zoom_center(1.25);
        }
        Key::Character(ref k) if cmd && k == "-" => {
            ev.prevent_default();
            controller::zoom_center(1.0 / 1.25);
        }
        Key::Character(ref k) if cmd && k == "0" => {
            ev.prevent_default();
            controller::zoom_reset();
        }
        Key::Delete | Key::Backspace => {
            ev.prevent_default();
            state::delete_selection();
        }
        _ => {}
    }
}
