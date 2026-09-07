//! Main-thread side of the structure-learning Web Worker: build the request
//! (names on the wire), spawn the worker, pump progress into `JOB_PROGRESS`,
//! poll the session's cancel flag, and translate the final message.
//!
//! The worker shim/glue/wasm are EMBEDDED in the app binary (dx only serves
//! manganis-referenced assets, and embedding keeps app and worker in
//! lockstep: re-running scripts/build-worker.sh changes the embedded bytes,
//! which makes cargo rebuild the app). They reach the Worker as Blob URLs.
//! The two build artifacts are gitignored; build.rs creates empty
//! placeholders so the crate compiles before the script has run — detected
//! here with a clear error.

use std::sync::atomic::Ordering;

use bn_core::io;
use bn_core::model::NodeId;
use bn_session::jobs::JobEvent;
use bn_session::ops::learn::StructureJobInput;
use bn_worker::proto::{StructureRequest, WorkerMsg, PROTO_VERSION};
use futures_channel::mpsc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Blob, BlobPropertyBag, MessageEvent, Url, Worker};

use super::web::{CaseFile, StructureOutcome};
use crate::state::JOB_PROGRESS;

const SHIM_JS: &str = include_str!("../../assets/worker/worker_shim.js");
const GLUE_JS: &str = include_str!("../../assets/worker/bn_worker.js");
const WORKER_WASM: &[u8] = include_bytes!("../../assets/worker/bn_worker_bg.wasm");

/// A same-origin blob URL for embedded content (must be revoked after use).
fn blob_url(bytes: &[u8], mime: &str) -> Result<String, String> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &opts)
        .map_err(|e| format!("cannot create blob: {e:?}"))?;
    Url::create_object_url_with_blob(&blob).map_err(|e| format!("cannot create blob url: {e:?}"))
}

fn build_request(input: &StructureJobInput, tsv: bool) -> Result<String, String> {
    let net = &input.net;
    let name = |id: NodeId| net.node(id).name.clone();
    let iodoc = io::Document { network: net.clone(), visual: Default::default() };
    let (net_json, _) = io::save_str(&iodoc, io::Format::NativeJson).map_err(|e| e.to_string())?;
    let o = &input.opts;
    let req = StructureRequest {
        proto_version: PROTO_VERSION,
        net_json,
        tsv,
        algo: o.algo,
        score: o.score,
        ess: o.ess,
        max_parents: o.max_parents,
        alpha: o.alpha,
        class_node: input.class.map(name),
        param_method: o.param_method,
        em_iters: o.em_iters,
        required_edges: o.required_edges.iter().map(|&(a, b)| (name(a), name(b))).collect(),
        forbidden_edges: o.forbidden_edges.iter().map(|&(a, b)| (name(a), name(b))).collect(),
    };
    serde_json::to_string(&req).map_err(|e| e.to_string())
}

pub async fn run(input: StructureJobInput, cases: CaseFile) -> Result<StructureOutcome, String> {
    if WORKER_WASM.is_empty() {
        return Err("the learning worker was not compiled in — run \
                    scripts/build-worker.sh and rebuild the app"
            .into());
    }
    let bytes = cases.read().await?;
    let tsv = cases.forced_delim() == Some(b'\t');
    let request = build_request(&input, tsv)?;

    let shim_url = blob_url(SHIM_JS.as_bytes(), "application/javascript")?;
    let glue_url = blob_url(GLUE_JS.as_bytes(), "application/javascript")?;
    let wasm_url = blob_url(WORKER_WASM, "application/wasm")?;
    let urls = [shim_url.clone(), glue_url.clone(), wasm_url.clone()];
    let revoke_all = move || {
        for u in &urls {
            let _ = Url::revoke_object_url(u);
        }
    };

    let worker = match Worker::new(&shim_url) {
        Ok(w) => w,
        Err(e) => {
            revoke_all();
            return Err(format!("cannot start worker: {e:?}"));
        }
    };

    // Worker messages → a channel we can poll alongside the cancel flag.
    let (tx, mut rx) = mpsc::unbounded::<WorkerMsg>();
    let tx_err = tx.clone();
    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let Some(s) = ev.data().as_string() else { return };
        let msg = serde_json::from_str::<WorkerMsg>(&s)
            .unwrap_or_else(|e| WorkerMsg::Failed { error: format!("bad worker message: {e}") });
        let _ = tx.unbounded_send(msg);
    });
    worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    let onerror = Closure::<dyn FnMut(JsValue)>::new(move |_| {
        let _ = tx_err.unbounded_send(WorkerMsg::Failed {
            error: "the learning worker failed to load".into(),
        });
    });
    worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));

    // Kick off: blob URLs + request JSON + case bytes (buffer transferred).
    let kickoff = (|| -> Result<(), String> {
        let msg = js_sys::Object::new();
        let set = |k: &str, v: &JsValue| {
            js_sys::Reflect::set(&msg, &JsValue::from_str(k), v)
                .map_err(|_| "cannot build message".to_string())
                .map(|_| ())
        };
        set("glue", &JsValue::from_str(&glue_url))?;
        set("wasm", &JsValue::from_str(&wasm_url))?;
        set("request", &JsValue::from_str(&request))?;
        let u8 = js_sys::Uint8Array::from(bytes.as_slice());
        set("cases", &u8)?;
        let transfer = js_sys::Array::of1(&u8.buffer());
        worker
            .post_message_with_transfer(&msg, &transfer)
            .map_err(|e| format!("cannot message worker: {e:?}"))
    })();
    if let Err(e) = kickoff {
        worker.terminate();
        revoke_all();
        return Err(e);
    }

    // Pump progress; poll the cancel flag (the Cancel button sets it through
    // ops::learn::cancel_structure_job, unchanged). Cancel = terminate — no
    // signal needs to reach the busy worker.
    let cancel = input.cancel.clone();
    let outcome = loop {
        match rx.try_recv() {
            Ok(WorkerMsg::Progress { frac, text }) => {
                *JOB_PROGRESS.write() = Some(JobEvent { frac, text });
                continue;
            }
            Ok(WorkerMsg::Done { patch, report, summary, warnings }) => {
                break StructureOutcome::Done { patch, report, summary, warnings };
            }
            Ok(WorkerMsg::Failed { error }) => break StructureOutcome::Failed(error),
            Err(mpsc::TryRecvError::Closed) => {
                break StructureOutcome::Failed("worker channel closed".into());
            }
            Err(mpsc::TryRecvError::Empty) => {} // no message pending
        }
        if cancel.load(Ordering::Relaxed) {
            break StructureOutcome::Cancelled;
        }
        super::web::sleep_ms(100).await;
    };
    worker.terminate();
    revoke_all();
    Ok(outcome)
}
