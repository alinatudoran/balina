//! AI provider integration (desktop only). Two configurable model roles:
//! a *text* model (generative: drafting, explanation) and a *structured*
//! model (constrained output: state probabilities, CPT rows). Settings are
//! global app state, never part of a document; calls go directly from the
//! app to the configured provider with the user's own API key.

pub mod client;
pub mod cpt;
pub mod definite;
pub mod draft;
pub mod evidence;
pub mod laya_local;
pub mod settings;
