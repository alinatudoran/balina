//! Structure learning off the browser's main thread: bn-core/bn-session
//! compiled into a standalone wasm module that runs inside a Web Worker.
//! The main app (`bn-app` web build) talks to it with the JSON messages in
//! [`proto`]; case bytes ride the postMessage as a transferred Uint8Array.
//!
//! Build with `scripts/build-worker.sh` (wasm-bindgen `--target no-modules`,
//! output under `crates/bn-app/assets/worker/`). Cancellation is
//! `worker.terminate()` from the main thread — no signal reaches the job.

pub mod proto;

#[cfg(target_arch = "wasm32")]
mod entry;
