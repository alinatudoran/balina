//! Op bodies, one `pub fn` per user-level action. The Dioxus UI calls each
//! of these through its session signal — exactly one op per user gesture.
//! All of this is testable with no UI runtime: build a [`crate::Session`],
//! call the functions.

pub mod cpt;
pub mod edit;
pub mod evidence;
pub mod file;
pub mod learn;
pub mod tools;
