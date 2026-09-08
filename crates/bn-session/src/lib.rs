//! Frontend-agnostic application session for the Balina editor.
//!
//! Owns the editable [`Document`] (network + visuals + evidence + snapshot
//! undo) and the [`EngineBridge`] (compiled engine + belief caches + dirty
//! tracking). The Dioxus UI holds a `Session` in a signal and calls the
//! functions in [`ops`] — exactly one op per user gesture — then re-renders
//! from the session state; there is no serialization boundary.

pub mod doc;
pub mod engine_bridge;
pub mod error;
pub mod jobs;
pub mod ops;
pub mod patch;
pub mod session;
pub mod views;

#[cfg(test)]
mod smoke_tests;

pub use doc::{Dirt, Document, NodeVisual, Point};
pub use engine_bridge::EngineBridge;
pub use error::CmdError;
pub use session::Session;
