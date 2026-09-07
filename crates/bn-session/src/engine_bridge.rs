//! Bridge between the editable document and the bn-core inference engine:
//! dirty tracking, belief cache, expected utilities.
//!
//! The egui app resynced once per frame; here the equivalent hook is
//! [`crate::Session::finish`], called at the end of every mutating command.

use bn_core::decision::expected_utilities;
use bn_core::decision::single::utility_expectation;
use bn_core::inference::Engine;
use bn_core::model::{Network, NodeId, NodeKind};
use bn_core::InferenceError;
use slotmap::SecondaryMap;

use crate::doc::{Dirt, Document};

#[derive(Default)]
pub struct EngineBridge {
    engine: Option<Engine>,
    pub beliefs: SecondaryMap<NodeId, Vec<f64>>,
    pub decision_eu: SecondaryMap<NodeId, Vec<Option<f64>>>,
    pub utility_ev: SecondaryMap<NodeId, f64>,
    pub log_p_e: Option<f64>,
    pub conflict: bool,
    pub compiled: bool,
    dirt: Dirt,
    pub last_compile_ms: f64,
}

impl EngineBridge {
    pub fn mark(&mut self, dirt: Dirt) {
        self.dirt = self.dirt.max(dirt);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirt != Dirt::None || !self.compiled
    }

    pub fn invalidate(&mut self) {
        self.engine = None;
        self.compiled = false;
        self.dirt = Dirt::Structure;
        self.beliefs.clear();
        self.decision_eu.clear();
        self.utility_ev.clear();
        self.log_p_e = None;
        self.conflict = false;
    }

    /// Borrow the compiled engine (for sensitivity, etc.). Recomputes first.
    pub fn engine_mut(&mut self, doc: &Document) -> &mut Engine {
        self.recompute(doc);
        self.engine.as_mut().expect("engine after recompute")
    }

    /// Synchronous recompute according to accumulated dirt.
    pub fn recompute(&mut self, doc: &Document) {
        let start = web_time::Instant::now();
        if self.engine.is_none() || self.dirt >= Dirt::Params {
            // Recompile covers both structure and CPT changes (compilation is
            // sub-millisecond at editor scale, not worth splitting).
            self.engine = Some(Engine::compile(&doc.net));
            self.compiled = true;
        }
        let engine = self.engine.as_mut().unwrap();
        engine.set_evidence(doc.evidence.clone());
        self.dirt = Dirt::None;
        match engine.all_beliefs() {
            Ok(all) => {
                self.conflict = false;
                self.beliefs = all;
                self.log_p_e = engine.log_prob_of_findings().ok();
            }
            Err(InferenceError::ConflictingEvidence) => {
                self.conflict = true;
                self.log_p_e = None;
            }
            Err(_) => {}
        }
        self.decision_eu.clear();
        self.utility_ev.clear();
        if !self.conflict && has_kind(&doc.net, NodeKind::Utility) {
            for (id, node) in doc.net.nodes() {
                match node.kind {
                    NodeKind::Decision => {
                        if let Ok(eu) = expected_utilities(engine, &doc.net, id) {
                            self.decision_eu.insert(id, eu);
                        }
                    }
                    NodeKind::Utility => {
                        if let Ok(ev) = utility_expectation(engine, &doc.net, id) {
                            self.utility_ev.insert(id, ev);
                        }
                    }
                    NodeKind::Chance => {}
                }
            }
        }
        self.last_compile_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

fn has_kind(net: &Network, kind: NodeKind) -> bool {
    net.nodes().any(|(_, n)| n.kind == kind)
}
