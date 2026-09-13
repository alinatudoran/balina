//! The per-window session: a project containing one or more tabs, each with
//! its own document + engine bridge + background-job slot.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use slotmap::SlotMap;

use crate::doc::{Dirt, Document};
use crate::engine_bridge::EngineBridge;

slotmap::new_key_type! {
    /// Stable key for a tab within a session. Does NOT survive serialization
    /// (tabs are reallocated fresh keys on load, like `NodeId` and `NoteId`).
    pub struct TabId;
}

/// One tab: a network document + its inference bridge + its background-job
/// cancel flag.
pub struct Tab {
    pub doc: Document,
    pub bridge: EngineBridge,
    /// Cancel flag of the in-flight background job (structure learning), if
    /// any. `Some` doubles as the "busy" marker.
    pub job_cancel: Option<Arc<AtomicBool>>,
}

impl Tab {
    pub fn new() -> Tab {
        Tab { doc: Document::new(), bridge: EngineBridge::default(), job_cancel: None }
    }
}

impl Default for Tab {
    fn default() -> Self {
        Tab::new()
    }
}

pub struct Session {
    tabs: SlotMap<TabId, Tab>,
    active: TabId,
    /// Insertion-order for tab bar rendering.
    order: Vec<TabId>,
    /// Project-level file path (the .balina that contains all tabs).
    pub path: Option<PathBuf>,
    /// Project-level dirty flag (set by any tab mutation or tab add/remove).
    pub modified: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        let mut tabs = SlotMap::with_key();
        let id = tabs.insert(Tab::new());
        Session { tabs, active: id, order: vec![id], path: None, modified: false }
    }

    // ---- active-tab accessors (used by all ops) ----------------------------

    pub fn doc(&self) -> &Document {
        &self.tabs[self.active].doc
    }
    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.tabs[self.active].doc
    }
    pub fn bridge(&self) -> &EngineBridge {
        &self.tabs[self.active].bridge
    }
    pub fn bridge_mut(&mut self) -> &mut EngineBridge {
        &mut self.tabs[self.active].bridge
    }
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    pub fn active_id(&self) -> TabId {
        self.active
    }

    // ---- tab navigation ----------------------------------------------------

    pub fn tab_order(&self) -> &[TabId] {
        &self.order
    }
    pub fn tab(&self, id: TabId) -> &Tab {
        &self.tabs[id]
    }
    pub fn tab_mut(&mut self, id: TabId) -> &mut Tab {
        &mut self.tabs[id]
    }
    pub fn tab_count(&self) -> usize {
        self.order.len()
    }
    pub fn tabs(&self) -> &SlotMap<TabId, Tab> {
        &self.tabs
    }

    // ---- tab label (defaults to doc.net.name) ------------------------------

    pub fn tab_label(&self, id: TabId) -> &str {
        &self.tabs[id].doc.net.name
    }

    // ---- finish hook -------------------------------------------------------

    /// Mark dirt and recompute if auto-update is on. Every mutating op ends
    /// here; the UI re-renders from the session state afterwards.
    pub fn finish(&mut self, dirt: Dirt) {
        let tab = &mut self.tabs[self.active];
        tab.bridge.mark(dirt);
        if tab.doc.auto_update && tab.bridge.is_dirty() && !tab.doc.net.is_empty() {
            tab.bridge.recompute(&tab.doc);
        }
        self.modified = true;
    }

    /// Finish for a specific tab (used when applying background job results to
    /// a non-active tab).
    pub fn finish_tab(&mut self, id: TabId, dirt: Dirt) {
        let tab = &mut self.tabs[id];
        tab.bridge.mark(dirt);
        if tab.doc.auto_update && tab.bridge.is_dirty() && !tab.doc.net.is_empty() {
            tab.bridge.recompute(&tab.doc);
        }
        self.modified = true;
    }

    // ---- tab mutations (used by ops::tab) ----------------------------------

    /// Insert a new tab after the active tab and return its id.
    pub fn insert_tab(&mut self, tab: Tab) -> TabId {
        let id = self.tabs.insert(tab);
        let pos = self.order.iter().position(|&t| t == self.active).unwrap_or(self.order.len());
        self.order.insert(pos + 1, id);
        id
    }

    /// Remove a tab. Returns the removed Tab. Panics if `id` is the only tab.
    pub fn remove_tab(&mut self, id: TabId) -> Tab {
        assert!(self.order.len() > 1, "cannot remove the last tab");
        self.order.retain(|&t| t != id);
        if self.active == id {
            self.active = self.order[0];
        }
        self.tabs.remove(id).expect("tab not found")
    }

    pub fn set_active(&mut self, id: TabId) {
        assert!(self.tabs.contains_key(id), "tab not found");
        self.active = id;
    }

    pub fn reorder_tab(&mut self, id: TabId, new_index: usize) {
        self.order.retain(|&t| t != id);
        let idx = new_index.min(self.order.len());
        self.order.insert(idx, id);
    }

    /// Reset to a fresh single-tab session (File > New).
    pub fn reset(&mut self) {
        self.tabs.clear();
        let id = self.tabs.insert(Tab::new());
        self.order = vec![id];
        self.active = id;
        self.path = None;
        self.modified = false;
    }

    /// Replace all tabs from a loaded project. Caller provides pre-built tabs.
    pub fn load_tabs(&mut self, tabs: Vec<Tab>, active_index: usize) {
        self.tabs.clear();
        self.order.clear();
        for tab in tabs {
            let id = self.tabs.insert(tab);
            self.order.push(id);
        }
        let idx = active_index.min(self.order.len().saturating_sub(1));
        self.active = self.order[idx];
        self.modified = false;
    }
}
