//! Transient canvas state: viewport + the gesture state machine.
//!
//! Forked from Dioxus/UI's `use_workflow` (rust-ui/dioxus-ui, MIT) and
//! gutted into a pure gesture/viewport controller: the document owns all
//! structure and positions; this owns ONLY transients (pan/zoom, in-flight
//! drag positions, the connect preview). Everything is keyed by `NodeId`,
//! never by index. Selection lives in `state::SELECTION`.

use std::collections::HashMap;

use bn_core::model::{NodeId, NodeKind};
use dioxus::prelude::*;

use crate::canvas::geometry::{Pt, Rect, Viewport};

/// Drag threshold in client px before a mousedown becomes a node drag
/// (React Flow's `nodeDragThreshold=1` — keeps clicks ≠ drags).
pub const DRAG_THRESHOLD: f64 = 1.0;

pub static VIEWPORT: GlobalSignal<Viewport> = Signal::global(Viewport::default);
pub static GESTURE: GlobalSignal<Gesture> = Signal::global(Gesture::default);
/// Optimistic world positions for nodes mid-drag.
pub static DRAG_POS: GlobalSignal<HashMap<NodeId, (f64, f64)>> = Signal::global(HashMap::new);
/// Set once a PendingNodeDrag crosses the threshold — the click that follows
/// a drag must not toggle evidence. Cleared on the next node mousedown.
pub static DRAG_HAPPENED: GlobalSignal<bool> = Signal::global(|| false);
/// Bump to make the canvas fit-view (doc load, toolbar, pane menu).
pub static FIT_REQUEST: GlobalSignal<u64> = Signal::global(|| 1);

#[derive(Clone, PartialEq, Debug)]
pub struct Hover {
    pub node: NodeId,
    pub valid: bool,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub enum Gesture {
    #[default]
    Idle,
    /// Armed on node mousedown; promoted to `DragNodes` past DRAG_THRESHOLD.
    PendingNodeDrag {
        start_client: (f64, f64),
        /// (id, start_x, start_y) world positions of every node that will move.
        starts: Vec<(NodeId, f64, f64)>,
    },
    DragNodes {
        start_client: (f64, f64),
        starts: Vec<(NodeId, f64, f64)>,
    },
    Pan {
        start_client: (f64, f64),
        pan_start: (f64, f64),
    },
    /// Rubber-band selection; coords in client space.
    RubberBand {
        start_client: (f64, f64),
        cur_client: (f64, f64),
        /// Additive (shift held at start): keeps the previous selection.
        additive: bool,
    },
    /// New-edge drag from `from`; preview follows `cursor_world`.
    Connect {
        from: NodeId,
        cursor_world: Pt,
        over: Option<Hover>,
    },
    /// Rewiring an existing edge's child end.
    Reconnect {
        parent: NodeId,
        orig_child: NodeId,
        cursor_world: Pt,
        over: Option<Hover>,
    },
    /// Palette ghost-drag: a node kind following the cursor (client coords).
    /// Handled at the app root, not the canvas (it starts in the toolbar).
    PaletteDrag {
        kind: NodeKind,
        start_client: (f64, f64),
        cur_client: (f64, f64),
    },
}

pub fn is_connecting() -> bool {
    matches!(*GESTURE.read(), Gesture::Connect { .. } | Gesture::Reconnect { .. })
}

/// Reset to Idle, dropping any optimistic positions (a cancelled drag must
/// land as no-op, never a half-commit).
pub fn cancel_gesture() {
    if !matches!(*GESTURE.read(), Gesture::Idle) {
        *GESTURE.write() = Gesture::Idle;
    }
    if !DRAG_POS.read().is_empty() {
        DRAG_POS.write().clear();
    }
}

/// Zoom by `factor` around the viewport center (toolbar buttons, hotkeys).
pub fn zoom_center(factor: f64) {
    let mut vp = VIEWPORT.write();
    let (cx, cy) = (vp.size.0 / 2.0, vp.size.1 / 2.0);
    vp.zoom_at_scale(cx, cy, factor);
}

/// Back to 100%, keeping the view center anchored.
pub fn zoom_reset() {
    let mut vp = VIEWPORT.write();
    let (cx, cy) = (vp.size.0 / 2.0, vp.size.1 / 2.0);
    let scale = 1.0 / vp.zoom;
    vp.zoom_at_scale(cx, cy, scale);
}

/// New drag positions for a node drag: world delta = client delta / zoom.
pub fn drag_update(
    starts: &[(NodeId, f64, f64)],
    start_client: (f64, f64),
    cur_client: (f64, f64),
    zoom: f64,
) -> HashMap<NodeId, (f64, f64)> {
    let dx = (cur_client.0 - start_client.0) / zoom;
    let dy = (cur_client.1 - start_client.1) / zoom;
    starts.iter().map(|&(id, sx, sy)| (id, (sx + dx, sy + dy))).collect()
}

/// World-space rect of a client-space rubber band.
pub fn rubber_band_world_rect(
    vp: &Viewport,
    start_client: (f64, f64),
    cur_client: (f64, f64),
) -> Rect {
    let a = vp.client_to_world(start_client.0, start_client.1);
    let b = vp.client_to_world(cur_client.0, cur_client.1);
    Rect::new(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs())
}

/// Validate `source → target` against the CURRENT session state (ancestor
/// sets recomputed on the spot — editor-scale nets make this trivial; hover
/// events are not per-frame).
pub fn check_link_now(
    source: NodeId,
    target: NodeId,
    reconnect_original_child: Option<NodeId>,
) -> crate::canvas::validation::LinkCheck {
    let s = crate::state::SESSION.read();
    let anc = bn_session::views::ancestor_sets(&s.doc.net);
    crate::canvas::validation::check_link(&s.doc.net, &anc, source, target, reconnect_original_child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::KeyData;

    fn nid(n: u64) -> NodeId {
        // Fabricated ids are fine for pure math tests.
        NodeId::from(KeyData::from_ffi((1 << 32) | n))
    }

    #[test]
    fn drag_update_scales_by_zoom() {
        let starts = vec![(nid(1), 100.0, 100.0), (nid(2), 200.0, 50.0)];
        let out = drag_update(&starts, (10.0, 10.0), (30.0, 0.0), 2.0);
        assert_eq!(out[&nid(1)], (110.0, 95.0));
        assert_eq!(out[&nid(2)], (210.0, 45.0));
    }

    #[test]
    fn rubber_band_rect_normalizes_corners() {
        let vp = Viewport { pan: (0.0, 0.0), zoom: 1.0, size: (800.0, 600.0), origin: (0.0, 0.0) };
        let r = rubber_band_world_rect(&vp, (100.0, 200.0), (50.0, 120.0));
        assert_eq!((r.x, r.y, r.w, r.h), (50.0, 120.0, 50.0, 80.0));
    }
}
