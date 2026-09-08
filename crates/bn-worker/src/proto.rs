//! Wire types between the app and the worker. Node references are NAMES:
//! NodeIds are slotmap keys and do not survive serialization (the worker
//! deserializes its own `Network` with fresh ids).

use bn_session::ops::learn::{LearnMethod, ScoreChoice, StructAlgo};
use bn_session::patch::StructurePatch;
use serde::{Deserialize, Serialize};

/// Bumped on any wire change; the worker refuses mismatched requests so a
/// stale build (dx rebuilt the app, nobody re-ran build-worker.sh) fails
/// loudly instead of misbehaving.
pub const PROTO_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructureRequest {
    pub proto_version: u32,
    /// The network in native `.balina` JSON (via `io::save_str`).
    pub net_json: String,
    /// True forces tab delimiting (`.tsv` case file).
    pub tsv: bool,
    pub algo: StructAlgo,
    pub score: ScoreChoice,
    pub ess: f64,
    pub max_parents: usize,
    pub alpha: f64,
    pub class_node: Option<String>,
    pub param_method: LearnMethod,
    pub em_iters: usize,
    pub required_edges: Vec<(String, String)>,
    pub forbidden_edges: Vec<(String, String)>,
}

/// Worker → app. Serialized as JSON strings over postMessage.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerMsg {
    Progress { frac: Option<f32>, text: String },
    Done { patch: StructurePatch, report: String, summary: String, warnings: Vec<String> },
    Failed { error: String },
}
