//! The editable document: network + visual layout + evidence + undo.
//! Undo is snapshot-based: editor-scale networks are small, and full
//! snapshots make every edit (including state remaps that reshape child
//! tables) trivially reversible.

use std::path::PathBuf;

use bn_core::inference::{Evidence, Finding};
use bn_core::io::{self, DisplayMode};
use bn_core::model::{Network, NodeId, NodeKind, State};
use slotmap::{SecondaryMap, SlotMap};

slotmap::new_key_type! {
    /// Key for sticky notes. Like `NodeId`, it does NOT survive
    /// serialization — notes are reallocated fresh keys on load.
    pub struct NoteId;
}

/// World-coordinate position (replaces egui's Pos2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }
}

#[derive(Clone, Debug)]
pub struct NodeVisual {
    pub pos: Point,
    pub display: DisplayMode,
    pub color: Option<[u8; 3]>,
}

impl Default for NodeVisual {
    fn default() -> Self {
        NodeVisual { pos: Point::default(), display: DisplayMode::BeliefBars, color: None }
    }
}

pub const NOTE_DEFAULT_SIZE: (f32, f32) = (200.0, 150.0);
pub const NOTE_MIN_SIZE: (f32, f32) = (80.0, 50.0);
/// Height of a collapsed note: just the title bar (macOS-Stickies style).
pub const NOTE_COLLAPSED_H: f32 = 24.0;
pub const NOTE_DEFAULT_COLOR: [u8; 3] = [255, 244, 165]; // sticky yellow
pub const NOTE_FONT_DEFAULT: f32 = 13.0;
pub const NOTE_FONT_RANGE: (f32, f32) = (8.0, 32.0);

/// A sticky note on the canvas. Pure annotation: lives outside the network,
/// so it can never affect inference or learning.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub text: String,
    pub pos: Point,
    pub w: f32,
    pub h: f32,
    pub color: [u8; 3],
    pub font_size: f32,
    /// Collapsed to just the title bar; `w`/`h` keep the expanded size.
    pub collapsed: bool,
}

impl Default for Note {
    fn default() -> Self {
        Note {
            text: String::new(),
            pos: Point::default(),
            w: NOTE_DEFAULT_SIZE.0,
            h: NOTE_DEFAULT_SIZE.1,
            color: NOTE_DEFAULT_COLOR,
            font_size: NOTE_FONT_DEFAULT,
            collapsed: false,
        }
    }
}

/// What a change invalidates in the inference engine.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Dirt {
    #[default]
    None,
    Evidence,
    Params,
    Structure,
}

impl Dirt {
    pub fn max(self, other: Dirt) -> Dirt {
        if other > self { other } else { self }
    }
}

#[derive(Clone)]
struct Snapshot {
    net: Network,
    visual: SecondaryMap<NodeId, NodeVisual>,
    notes: SlotMap<NoteId, Note>,
    evidence: Evidence,
}

pub struct Document {
    pub net: Network,
    pub visual: SecondaryMap<NodeId, NodeVisual>,
    pub notes: SlotMap<NoteId, Note>,
    pub evidence: Evidence,
    pub auto_update: bool,
    pub path: Option<PathBuf>,
    pub modified: bool,
    /// Model generation counter: bumped by every change that could affect
    /// net/evidence (but not by visual-only edits). Background jobs record
    /// it at spawn and discard their result on mismatch.
    pub change_seq: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

const UNDO_CAP: usize = 100;

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

impl Document {
    pub fn new() -> Document {
        Document {
            net: Network::new("Untitled"),
            visual: SecondaryMap::new(),
            notes: SlotMap::with_key(),
            evidence: Evidence::new(),
            auto_update: true,
            path: None,
            modified: false,
            change_seq: 0,
            undo: vec![],
            redo: vec![],
        }
    }

    // ---- undo ----------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            net: self.net.clone(),
            visual: self.visual.clone(),
            notes: self.notes.clone(),
            evidence: self.evidence.clone(),
        }
    }

    /// Call before mutating net/visual/evidence as one undoable step.
    pub fn begin_change(&mut self) {
        self.change_seq += 1;
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
    }

    /// Snapshot for undo WITHOUT bumping the model generation counter — for
    /// visual-only edits (node moves, display mode) that can't invalidate an
    /// in-flight background job's result.
    pub fn begin_visual_change(&mut self) {
        let seq = self.change_seq;
        self.begin_change();
        self.change_seq = seq;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> Dirt {
        if let Some(s) = self.undo.pop() {
            self.change_seq += 1;
            self.redo.push(self.snapshot());
            self.restore(s);
            Dirt::Structure
        } else {
            Dirt::None
        }
    }

    pub fn redo(&mut self) -> Dirt {
        if let Some(s) = self.redo.pop() {
            self.change_seq += 1;
            self.undo.push(self.snapshot());
            self.restore(s);
            Dirt::Structure
        } else {
            Dirt::None
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.net = s.net;
        self.visual = s.visual;
        self.notes = s.notes;
        self.evidence = s.evidence;
        self.modified = true;
    }

    // ---- edits -----------------------------------------------------------

    /// Auto-name: A, B, ..., Z, N27, N28, ...
    pub fn fresh_name(&self, kind: NodeKind) -> String {
        let prefix = match kind {
            NodeKind::Chance => "",
            NodeKind::Decision => "D",
            NodeKind::Utility => "U",
        };
        for i in 0..26u32 {
            let c = char::from(b'A' + i as u8);
            let name = format!("{prefix}{c}");
            if self.net.find_by_name(&name).is_none() {
                return name;
            }
        }
        let mut i = 27;
        loop {
            let name = format!("{prefix}N{i}");
            if self.net.find_by_name(&name).is_none() {
                return name;
            }
            i += 1;
        }
    }

    pub fn add_node_at(&mut self, kind: NodeKind, pos: Point) -> Result<NodeId, bn_core::ModelError> {
        self.begin_change();
        let name = self.fresh_name(kind);
        let states = match kind {
            NodeKind::Utility => vec![],
            _ => vec![State::new("state0"), State::new("state1")],
        };
        let id = self.net.add_node(&name, kind, states)?;
        let display = match kind {
            NodeKind::Utility => DisplayMode::ExpectedValue,
            _ => DisplayMode::BeliefBars,
        };
        self.visual.insert(id, NodeVisual { pos, display, color: None });
        Ok(id)
    }

    /// Add a sticky note. Visual-only: undoable but no `change_seq` bump.
    pub fn add_note_at(&mut self, pos: Point) -> NoteId {
        self.begin_visual_change();
        self.notes.insert(Note { pos, ..Default::default() })
    }

    /// Give any node missing a visual (e.g. created by CSV import) a default
    /// grid position below the existing layout. Caller has begin_change()'d.
    pub fn ensure_visuals(&mut self) {
        let max_y = self
            .net
            .node_ids()
            .iter()
            .filter_map(|&id| self.visual.get(id))
            .map(|v| v.pos.y)
            .fold(0.0f32, f32::max);
        let mut i = 0usize;
        for id in self.net.node_ids() {
            if !self.visual.contains_key(id) {
                let pos = Point::new(
                    60.0 + (i % 4) as f32 * 240.0,
                    max_y + 140.0 + (i / 4) as f32 * 140.0,
                );
                self.visual.insert(id, NodeVisual { pos, ..Default::default() });
                i += 1;
            }
        }
    }

    /// Delete edges first, then nodes (retracting their evidence), then
    /// notes; one undoable step. Replaces the egui app's selection-based
    /// deletion — selection now lives in the frontend. A notes-only delete
    /// is a visual change (no `change_seq` bump, nothing to recompute).
    pub fn delete_items(
        &mut self,
        nodes: &[NodeId],
        edges: &[(NodeId, NodeId)],
        notes: &[NoteId],
    ) -> Dirt {
        if nodes.is_empty() && edges.is_empty() && notes.is_empty() {
            return Dirt::None;
        }
        let dirt = if nodes.is_empty() && edges.is_empty() {
            self.begin_visual_change();
            Dirt::None
        } else {
            self.begin_change();
            Dirt::Structure
        };
        for &(a, b) in edges {
            let _ = self.net.remove_edge(a, b);
        }
        for &id in nodes {
            self.evidence.retract(id);
            self.net.remove_node(id);
        }
        for &id in notes {
            self.notes.remove(id);
        }
        dirt
    }

    pub fn toggle_finding(&mut self, node: NodeId, state: usize) -> Dirt {
        self.begin_change();
        match self.evidence.get(node) {
            Some(Finding::Hard(s)) if *s == state => self.evidence.retract(node),
            _ => self.evidence.set(node, Finding::Hard(state)),
        }
        Dirt::Evidence
    }

    pub fn set_likelihood_finding(&mut self, node: NodeId, values: Vec<f64>) -> Dirt {
        self.begin_change();
        self.evidence.set(node, Finding::Likelihood(values));
        Dirt::Evidence
    }

    pub fn retract_finding(&mut self, node: NodeId) -> Dirt {
        self.begin_change();
        self.evidence.retract(node);
        Dirt::Evidence
    }

    pub fn retract_all_findings(&mut self) -> Dirt {
        if self.evidence.is_empty() {
            return Dirt::None;
        }
        self.begin_change();
        self.evidence.clear();
        Dirt::Evidence
    }

    // ---- persistence -------------------------------------------------------

    pub fn to_io_document(&self) -> io::Document {
        let mut visual = io::VisualInfo::default();
        for (id, v) in self.visual.iter() {
            if let Some(node) = self.net.get(id) {
                visual.nodes.insert(
                    node.name.clone(),
                    io::NodeVisual { x: v.pos.x, y: v.pos.y, display: v.display, color: v.color },
                );
            }
        }
        for n in self.notes.values() {
            visual.notes.push(io::NoteInfo {
                text: n.text.clone(),
                x: n.pos.x,
                y: n.pos.y,
                w: n.w,
                h: n.h,
                color: n.color,
                font_size: n.font_size,
                collapsed: n.collapsed,
            });
        }
        io::Document { network: self.net.clone(), visual }
    }

    pub fn from_io_document(iodoc: io::Document, path: Option<PathBuf>) -> Document {
        let mut doc = Document::new();
        doc.net = iodoc.network;
        doc.path = path;
        // Layout: use stored positions; auto-layout the rest by topo depth.
        let mut depth: SecondaryMap<NodeId, usize> = SecondaryMap::new();
        let mut count_at: Vec<usize> = vec![];
        for id in doc.net.topo_order() {
            let d = doc
                .net
                .node(id)
                .parents
                .iter()
                .map(|&p| depth.get(p).copied().unwrap_or(0) + 1)
                .max()
                .unwrap_or(0);
            depth.insert(id, d);
            if count_at.len() <= d {
                count_at.resize(d + 1, 0);
            }
            let row = count_at[d];
            count_at[d] += 1;
            let stored = iodoc.visual.nodes.get(&doc.net.node(id).name);
            let v = match stored {
                Some(s) => NodeVisual {
                    pos: Point::new(s.x, s.y),
                    display: s.display,
                    color: s.color,
                },
                None => NodeVisual {
                    pos: Point::new(60.0 + d as f32 * 240.0, 60.0 + row as f32 * 140.0),
                    display: DisplayMode::BeliefBars,
                    color: None,
                },
            };
            doc.visual.insert(id, v);
        }
        for n in &iodoc.visual.notes {
            doc.notes.insert(Note {
                text: n.text.clone(),
                pos: Point::new(n.x, n.y),
                w: n.w.max(NOTE_MIN_SIZE.0),
                h: n.h.max(NOTE_MIN_SIZE.1),
                color: n.color,
                font_size: n.font_size.clamp(NOTE_FONT_RANGE.0, NOTE_FONT_RANGE.1),
                collapsed: n.collapsed,
            });
        }
        doc
    }
}
