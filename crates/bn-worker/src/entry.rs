//! The worker-side entry point: parse the request, run the job body, post
//! progress and the final by-name patch back to the main thread.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bn_core::io;
use bn_core::model::{Network, NodeId};
use bn_session::jobs::{JobCtx, JobEvent};
use bn_session::ops::learn::{self, StructureJobInput, StructureLearnOpts, StructureOutcome};
use bn_session::patch::structure_patch;
use wasm_bindgen::prelude::*;
use web_sys::DedicatedWorkerGlobalScope;

use crate::proto::{StructureRequest, WorkerMsg, PROTO_VERSION};

/// postMessage works from a blocked worker: messages flush to the main
/// thread even while `run_structure_job` keeps the worker busy.
fn post(msg: &WorkerMsg) {
    let json = serde_json::to_string(msg).expect("WorkerMsg serializes");
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let _ = scope.post_message(&JsValue::from_str(&json));
}

fn fail(error: impl Into<String>) {
    post(&WorkerMsg::Failed { error: error.into() });
}

fn resolve(net: &Network, name: &str) -> Result<NodeId, String> {
    net.find_by_name(name).ok_or_else(|| format!("unknown node `{name}` in request"))
}

fn resolve_edges(net: &Network, edges: &[(String, String)]) -> Result<Vec<(NodeId, NodeId)>, String> {
    edges.iter().map(|(a, b)| Ok((resolve(net, a)?, resolve(net, b)?))).collect()
}

#[wasm_bindgen]
pub fn worker_entry(request_json: String, cases: js_sys::Uint8Array) {
    // A Rust panic in wasm is an opaque `unreachable` trap — report the
    // message through the protocol first.
    std::panic::set_hook(Box::new(|info| fail(format!("worker panicked: {info}"))));
    let req: StructureRequest = match serde_json::from_str(&request_json) {
        Ok(r) => r,
        Err(e) => return fail(format!("bad request: {e}")),
    };
    if req.proto_version != PROTO_VERSION {
        return fail(format!(
            "worker/app protocol mismatch (worker {PROTO_VERSION}, app {}) — \
             rebuild with scripts/build-worker.sh",
            req.proto_version
        ));
    }
    let net = match io::load_str(&req.net_json, io::Format::NativeJson) {
        Ok(doc) => doc.network,
        Err(e) => return fail(format!("bad network: {e}")),
    };
    let opts = match (|| -> Result<StructureLearnOpts, String> {
        Ok(StructureLearnOpts {
            algo: req.algo,
            score: req.score,
            ess: req.ess,
            max_parents: req.max_parents,
            alpha: req.alpha,
            class_node: req.class_node.as_deref().map(|n| resolve(&net, n)).transpose()?,
            param_method: req.param_method,
            em_iters: req.em_iters,
            required_edges: resolve_edges(&net, &req.required_edges)?,
            forbidden_edges: resolve_edges(&net, &req.forbidden_edges)?,
        })
    })() {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let input = StructureJobInput {
        net: net.clone(),
        started_seq: 0, // staleness is checked app-side against the live session
        cancel: Arc::new(AtomicBool::new(false)), // cancel = terminate()
        class: opts.class_node,
        opts,
    };
    let jc = JobCtx::new(input.cancel.clone(), |e: JobEvent| {
        post(&WorkerMsg::Progress { frac: e.frac, text: e.text });
    });
    let delim = req.tsv.then_some(b'\t');
    match learn::run_structure_job(input, &cases.to_vec(), delim, &jc) {
        StructureOutcome::Done { net: learned, report, summary, warnings } => {
            post(&WorkerMsg::Done {
                patch: structure_patch(&net, &learned),
                report,
                summary,
                warnings,
            });
        }
        // Unreachable (no cancel signal enters the worker), but keep it typed.
        StructureOutcome::Cancelled => fail("cancelled"),
        StructureOutcome::Failed(e) => fail(e),
    }
}
