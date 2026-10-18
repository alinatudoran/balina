//! Dialogs. One open at a time (`state::DIALOG`); node-bound dialogs close
//! automatically when their node vanishes (undo, delete, learning).

#[cfg(not(target_arch = "wasm32"))]
pub mod ai_draft;
#[cfg(not(target_arch = "wasm32"))]
pub mod ai_evidence;
#[cfg(not(target_arch = "wasm32"))]
pub mod ai_settings;
pub mod arc_strength;
pub mod cpt_editor;
pub mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod laya_download;
pub mod learn_cpts;
pub mod misc;
pub mod node_props;
#[cfg(target_arch = "wasm32")]
pub mod save_web;
pub mod sensitivity;
pub mod simulate;
pub mod structure_learn;
