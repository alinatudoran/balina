//! The node-editor canvas.
//!
//! Layering (mirrors the old React store separation):
//! - [`geometry`], [`scene`], [`validation`] — pure, headless-testable math.
//! - `controller` — transient gesture/viewport state (fork of Dioxus/UI's
//!   `use_workflow`), never document state.
//! - components (`canvas`, `node`, `edge`, …) — render from `SESSION` +
//!   controller signals; commit gestures as single session ops.

#[allow(clippy::module_inception)]
pub mod canvas;
pub mod context_menu;
pub mod controller;
pub mod edge;
pub mod geometry;
pub mod minimap;
pub mod node;
pub mod preview;
pub mod scene;
pub mod sticky;
pub mod validation;
