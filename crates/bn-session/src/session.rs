//! The per-window session: document + engine bridge + the (single) slot for
//! an in-flight background job's cancel flag.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::doc::{Dirt, Document};
use crate::engine_bridge::EngineBridge;

#[derive(Default)]
pub struct Session {
    pub doc: Document,
    pub bridge: EngineBridge,
    /// Cancel flag of the in-flight background job (structure learning), if
    /// any. `Some` doubles as the "busy" marker.
    pub job_cancel: Option<Arc<AtomicBool>>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Mark dirt and recompute if auto-update is on. Every mutating op ends
    /// here (the moral equivalent of the egui app's per-frame `frame_sync`);
    /// the UI re-renders from the session state afterwards.
    pub fn finish(&mut self, dirt: Dirt) {
        self.bridge.mark(dirt);
        if self.doc.auto_update && self.bridge.is_dirty() && !self.doc.net.is_empty() {
            self.bridge.recompute(&self.doc);
        }
    }
}
