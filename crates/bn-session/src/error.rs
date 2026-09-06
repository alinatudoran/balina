//! Op errors surfaced to the UI (typically via the message log).
//!
//! `ConflictingEvidence` raised during a routine recompute is NOT an error —
//! it surfaces as `EngineBridge::conflict = true` after a successful
//! recompute. Only tools that cannot proceed at all (ID solving,
//! sensitivity) return inference errors here.

#[derive(Debug, Clone, thiserror::Error)]
pub enum CmdError {
    #[error("{0}")]
    Model(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Learn(String),
    #[error("{0}")]
    Inference(String),
    #[error("{0}")]
    Id(String),
    /// Unknown/stale node id or bad parameters from the UI.
    #[error("{0}")]
    BadRequest(String),
    /// A background job finished but the network changed while it ran.
    #[error("{0}")]
    Stale(String),
    /// A background job is already running.
    #[error("{0}")]
    Busy(String),
    #[error("cancelled")]
    Cancelled,
}

impl From<bn_core::ModelError> for CmdError {
    fn from(e: bn_core::ModelError) -> Self {
        CmdError::Model(e.to_string())
    }
}

impl From<bn_core::IoError> for CmdError {
    fn from(e: bn_core::IoError) -> Self {
        CmdError::Io(e.to_string())
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        CmdError::Io(e.to_string())
    }
}

impl From<bn_core::CaseError> for CmdError {
    fn from(e: bn_core::CaseError) -> Self {
        CmdError::Learn(e.to_string())
    }
}

impl From<bn_core::LearnError> for CmdError {
    fn from(e: bn_core::LearnError) -> Self {
        match e {
            bn_core::LearnError::Cancelled => CmdError::Cancelled,
            other => CmdError::Learn(other.to_string()),
        }
    }
}

impl From<bn_core::InferenceError> for CmdError {
    fn from(e: bn_core::InferenceError) -> Self {
        CmdError::Inference(e.to_string())
    }
}

impl From<bn_core::IdError> for CmdError {
    fn from(e: bn_core::IdError) -> Self {
        CmdError::Id(e.to_string())
    }
}
