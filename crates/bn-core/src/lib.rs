//! bn-core: a GUI-independent Bayesian network engine.
//!
//! Discrete Bayesian networks and influence diagrams with exact junction-tree
//! inference (hard + likelihood evidence), decision analysis, parameter
//! learning (counting + EM), sampling, sensitivity to findings, and file I/O
//! (native JSON, XMLBIF, XDSL).

pub mod decision;
pub mod error;
pub mod factor;
pub mod inference;
pub mod io;
pub mod learn;
pub mod model;
pub mod sample;
pub mod sensitivity;

pub use error::{CaseError, IdError, InferenceError, IoError, LearnError, ModelError};
pub use learn::structure::EdgeConstraints;
pub use factor::{Factor, VarId};
pub use inference::{posterior_enumeration, Engine, Evidence, Finding};
pub use model::{ContinuousInfo, Network, Node, NodeId, NodeKind, State, Table};
