//! Global signals + the single mutation gateway.
//!
//! `SESSION` is the in-process equivalent of the old Tauri
//! `State<Mutex<Session>>`: one coarse signal, every mutation goes through
//! [`exec`] with exactly one `ops::` call per user gesture. Transient UI
//! state (selection, dialogs, messages) lives in separate globals and never
//! enters the document.
//!
//! Borrow discipline: never call `exec` (or otherwise write a global) while
//! holding a `.read()` guard on the same signal — copy the `NodeId`s out
//! first. `SESSION.write()` guards must never live across an `.await`.

use std::collections::HashSet;

use bn_core::model::NodeId;
use bn_session::{CmdError, Session};
use dioxus::prelude::*;

pub static SESSION: GlobalSignal<Session> = Signal::global(Session::new);

/// Canvas selection — transient, id-keyed, never part of the document.
#[derive(Clone, Default, PartialEq)]
pub struct Selection {
    pub nodes: HashSet<NodeId>,
    /// Edges as (parent, child).
    pub edges: HashSet<(NodeId, NodeId)>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

pub static SELECTION: GlobalSignal<Selection> = Signal::global(Selection::default);

pub fn select_all() {
    let ids: HashSet<NodeId> = SESSION.read().doc.net.node_ids().into_iter().collect();
    *SELECTION.write() = Selection { nodes: ids, edges: HashSet::new() };
}

pub fn clear_selection() {
    let mut sel = SELECTION.write();
    if !sel.is_empty() {
        *sel = Selection::default();
    }
}

/// Drop selected items that no longer exist (after undo/delete/learning).
pub fn prune_selection() {
    let (live_nodes, live_edges) = {
        let s = SESSION.read();
        let sel = SELECTION.read();
        let live_nodes: HashSet<NodeId> =
            sel.nodes.iter().copied().filter(|&id| s.doc.net.contains(id)).collect();
        let edges = s.doc.net.edges();
        let live_edges: HashSet<(NodeId, NodeId)> =
            sel.edges.iter().copied().filter(|e| edges.contains(e)).collect();
        if live_nodes.len() == sel.nodes.len() && live_edges.len() == sel.edges.len() {
            return; // nothing stale; avoid a pointless signal write
        }
        (live_nodes, live_edges)
    };
    *SELECTION.write() = Selection { nodes: live_nodes, edges: live_edges };
}

/// Delete the current selection as one op (menu item, Delete hotkey).
pub fn delete_selection() {
    prune_selection();
    let (nodes, edges) = {
        let sel = SELECTION.read();
        if sel.is_empty() {
            return;
        }
        (
            sel.nodes.iter().copied().collect::<Vec<_>>(),
            sel.edges.iter().copied().collect::<Vec<_>>(),
        )
    };
    clear_selection();
    exec(|s| bn_session::ops::edit::delete_items(s, &nodes, &edges));
}

/// Message log (cap 300, matching the React store).
pub static MESSAGES: GlobalSignal<Vec<String>> = Signal::global(Vec::new);
pub static SHOW_MESSAGES: GlobalSignal<bool> = Signal::global(|| false);

/// True while a text input has focus — edit hotkeys are ignored (the
/// not-while-typing guard; set by the vendored Input/Textarea components).
pub static TYPING: GlobalSignal<bool> = Signal::global(|| false);

/// The single open dialog (single-slot, like the React store).
pub static DIALOG: GlobalSignal<Option<DialogDesc>> = Signal::global(|| None);

/// The open context menu (node / edge / pane).
pub static CONTEXT_MENU: GlobalSignal<Option<ContextMenuState>> = Signal::global(|| None);

#[derive(Clone, PartialEq)]
pub enum ContextMenuTarget {
    Node(NodeId),
    /// (parent, child).
    Edge(NodeId, NodeId),
    /// Pane click, with the world coordinates under the cursor.
    Pane { world: (f64, f64) },
}

#[derive(Clone, PartialEq)]
pub struct ContextMenuState {
    pub target: ContextMenuTarget,
    /// Anchor in client coordinates.
    pub client: (f64, f64),
}

/// Last case file picked by the learn dialogs (its directory pre-fills the
/// next picker on desktop; the handle stays readable on web).
pub static LAST_CASE_FILE: GlobalSignal<Option<crate::platform::CaseFile>> =
    Signal::global(|| None);

/// Progress of the in-flight background job (structure learning).
pub static JOB_PROGRESS: GlobalSignal<Option<bn_session::jobs::JobEvent>> =
    Signal::global(|| None);

#[derive(Clone, PartialEq)]
pub enum DialogDesc {
    NodeProperties { node: NodeId },
    CptEditor { node: NodeId },
    Likelihood { node: NodeId },
    LearnCpts,
    StructureLearn,
    Simulate,
    Sensitivity,
    ArcStrength,
    IdSolution { text: String },
    RenameNetwork,
    About,
    /// Web only: filename + format picker for the save-as-download flow.
    #[cfg(target_arch = "wasm32")]
    SaveAsWeb,
}

pub fn open_dialog(d: DialogDesc) {
    *DIALOG.write() = Some(d);
}

pub fn close_dialog() {
    *DIALOG.write() = None;
}

pub fn log_message(msg: impl Into<String>) {
    let mut m = MESSAGES.write();
    m.push(msg.into());
    if m.len() > 300 {
        let overflow = m.len() - 300;
        m.drain(..overflow);
    }
}

/// Run ONE user action against the session. The write guard is scoped to
/// this call — it never lives across an `.await`. Errors land in the
/// message log (same behavior as the old command wrapper).
pub fn exec<T>(f: impl FnOnce(&mut Session) -> Result<T, CmdError>) -> Option<T> {
    match exec_res(f) {
        Ok(t) => Some(t),
        Err(e) => {
            log_message(e.to_string());
            None
        }
    }
}

/// Like [`exec`] but hands the error back (dialogs that display it inline).
pub fn exec_res<T>(f: impl FnOnce(&mut Session) -> Result<T, CmdError>) -> Result<T, CmdError> {
    let r = f(&mut SESSION.write());
    // Any op can invalidate ids the transient state holds (undo, delete,
    // structure learning) — prune stale selection entries right away.
    prune_selection();
    r
}
