use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("duplicate node name `{0}`")]
    DuplicateName(String),
    #[error("invalid node name `{0}`")]
    InvalidName(String),
    #[error("adding link would create a cycle")]
    WouldCreateCycle,
    #[error("link already exists")]
    DuplicateEdge,
    #[error("no such link")]
    NoSuchEdge,
    #[error("table has wrong size: expected {expected}, got {got}")]
    TableShape { expected: usize, got: usize },
    #[error("node has no states")]
    NoStates,
    #[error("utility nodes cannot have children")]
    UtilityWithChildren,
    #[error("invalid state remap")]
    InvalidRemap,
    #[error("unknown node `{0}`")]
    UnknownNode(String),
}

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("evidence is conflicting: P(findings) = 0")]
    ConflictingEvidence,
    #[error("node is not a probabilistic variable (utility node)")]
    NotAVariable,
    #[error("likelihood vector has wrong length")]
    BadLikelihood,
    #[error("state index out of range")]
    BadState,
}

#[derive(Debug, Error)]
pub enum IoError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("xml error: {0}")]
    Xml(String),
    #[error("unsupported file format `{0}`")]
    UnknownFormat(String),
    #[error("bad file: {0}")]
    Malformed(String),
    #[error("model error: {0}")]
    Model(#[from] ModelError),
}

#[derive(Debug, Error)]
pub enum CaseError {
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("column `{0}` does not match any node")]
    UnknownColumn(String),
    #[error("value `{value}` is not a state of node `{node}`")]
    UnknownState { node: String, value: String },
    #[error("no columns match any node in the network")]
    NoMatchingColumns,
    #[error("column `{column}` has more than {max} distinct values; not usable as a discrete node")]
    TooManyStates { column: String, max: usize },
    #[error("model error: {0}")]
    Model(#[from] ModelError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum LearnError {
    #[error(transparent)]
    Case(#[from] CaseError),
    #[error(transparent)]
    Inference(#[from] InferenceError),
    #[error("network has no learnable (chance) nodes in the case set")]
    NothingToLearn,
    #[error("operation cancelled")]
    Cancelled,
    #[error("node `{0}` has no column in the case data")]
    NoDataColumn(String),
    #[error("node `{0}` is not a chance node")]
    NotChance(String),
    #[error("structure learning needs at least {min} nodes, got {got}")]
    TooFewVariables { min: usize, got: usize },
    #[error("contingency table too large ({cells} cells > {max})")]
    TableTooLarge { cells: usize, max: usize },
    #[error("model error: {0}")]
    Model(#[from] ModelError),
    #[error("invalid edge constraint: {0}")]
    BadConstraint(String),
}

#[derive(Debug, Error)]
pub enum IdError {
    #[error("network has no decision nodes")]
    NoDecisions,
    #[error("network has no utility nodes")]
    NoUtilities,
    #[error(transparent)]
    Inference(#[from] InferenceError),
}
