//! Balina desktop app: Dioxus 0.7 (webview) over the in-process
//! `bn-session` engine. No IPC — components call session ops directly
//! through the `SESSION` signal.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod chrome;
mod dialogs;
mod export;
mod logic;
mod state;
mod ui;

use std::path::PathBuf;
use std::sync::OnceLock;

use dioxus::desktop::{Config, WindowBuilder};

/// File passed on the command line, opened on first mount.
pub static INITIAL_FILE: OnceLock<PathBuf> = OnceLock::new();

fn main() {
    if let Some(arg) = std::env::args().nth(1) {
        let _ = INITIAL_FILE.set(PathBuf::from(arg));
    }
    let window = WindowBuilder::new()
        .with_title("Balina")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1200.0, 800.0));
    let config = Config::new()
        .with_window(window)
        .with_menu(chrome::menu::build());
    dioxus::LaunchBuilder::desktop().with_cfg(config).launch(app::App);
}
