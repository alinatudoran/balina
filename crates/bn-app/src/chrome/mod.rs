//! Application chrome: menu bar, toolbar, status bar, message log, file ops.

#[cfg(not(target_arch = "wasm32"))]
pub mod file_ops;
#[cfg(target_arch = "wasm32")]
#[path = "file_ops_web.rs"]
pub mod file_ops;
pub mod hotkeys;
pub mod menu_bar;
pub mod menu;
pub mod message_log;
#[cfg(not(target_arch = "wasm32"))]
pub mod recent;
pub mod status_bar;
pub mod toolbar;
