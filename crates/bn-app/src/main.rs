//! Balina app: Dioxus 0.7 over the in-process `bn-session` engine. No IPC —
//! components call session ops directly through the `SESSION` signal.
//! Compiles as a desktop app (webview, default) or a browser app (wasm32,
//! `--features web`); platform differences live in `platform`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod chrome;
mod dialogs;
mod export;
mod logic;
mod platform;
mod state;
mod ui;

/// File passed on the command line, opened on first mount (desktop only).
#[cfg(not(target_arch = "wasm32"))]
pub static INITIAL_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use dioxus::desktop::{Config, WindowBuilder};

    if let Some(arg) = std::env::args().nth(1) {
        let _ = INITIAL_FILE.set(std::path::PathBuf::from(arg));
    }
    let window = WindowBuilder::new()
        .with_title("Balina")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1200.0, 800.0));
    let config = Config::new()
        .with_window(window)
        .with_menu(chrome::menu::build());
    dioxus::LaunchBuilder::desktop().with_cfg(config).launch(app::App);
}

#[cfg(target_arch = "wasm32")]
fn main() {
    dioxus::launch(app::App);
}
