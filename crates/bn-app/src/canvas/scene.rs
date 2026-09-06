//! Pure projection: document (+ in-flight drag overrides) → world-space
//! geometry. Recomputed per render via `use_memo`; the document stays the
//! single source of truth for structure and positions.

use std::collections::HashMap;

use bn_core::io::DisplayMode;
use bn_core::model::{NodeId, NodeKind};
use bn_session::Document;
use slotmap::SecondaryMap;

use crate::canvas::geometry::{edge_endpoints, node_size, Pt, Rect};

#[derive(Clone, PartialEq, Debug)]
pub struct NodeGeom {
    pub id: NodeId,
    pub rect: Rect,
}

#[derive(Clone, PartialEq, Debug)]
pub struct EdgeGeom {
    /// (parent, child).
    pub key: (NodeId, NodeId),
    pub from: Pt,
    pub to: Pt,
    /// Child is a decision node → drawn dashed.
    pub informational: bool,
}

#[derive(Clone, PartialEq, Default, Debug)]
pub struct Scene {
    pub nodes: Vec<NodeGeom>,
    pub edges: Vec<EdgeGeom>,
}

impl Scene {
    pub fn rect_of(&self, id: NodeId) -> Option<Rect> {
        self.nodes.iter().find(|n| n.id == id).map(|n| n.rect)
    }

    /// Bounding box of all nodes (None when empty).
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.nodes.iter();
        let first = it.next()?.rect;
        Some(it.fold(first, |acc, n| acc.union(&n.rect)))
    }
}

/// Build world-space geometry for every node and edge. `drag_pos` holds
/// optimistic positions for nodes mid-drag (they override the document's
/// visual position until the drag commits one `move_nodes` op).
pub fn build_scene(doc: &Document, drag_pos: &HashMap<NodeId, (f64, f64)>) -> Scene {
    let mut rects: SecondaryMap<NodeId, Rect> = SecondaryMap::new();
    let mut nodes = Vec::with_capacity(doc.net.len());
    for id in doc.net.node_ids() {
        let n = doc.net.node(id);
        let v = doc.visual.get(id);
        let display = v.map(|v| v.display).unwrap_or_default();
        let title = if n.title.is_empty() { &n.name } else { &n.title };
        let has_stats_row = n.kind == NodeKind::Chance
            && display == DisplayMode::BeliefBars
            && n.states.iter().all(|s| s.value.is_some());
        let (w, h) = node_size(n.kind, display, title, n.states.len(), has_stats_row);
        let (x, y) = match drag_pos.get(&id) {
            Some(&(x, y)) => (x, y),
            None => v.map(|v| (v.pos.x as f64, v.pos.y as f64)).unwrap_or((0.0, 0.0)),
        };
        let rect = Rect::new(x, y, w, h);
        rects.insert(id, rect);
        nodes.push(NodeGeom { id, rect });
    }

    let edges = doc
        .net
        .edges()
        .into_iter()
        .filter_map(|(p, c)| {
            let (from, to) = edge_endpoints(*rects.get(p)?, *rects.get(c)?);
            Some(EdgeGeom {
                key: (p, c),
                from,
                to,
                informational: doc.net.node(c).kind == NodeKind::Decision,
            })
        })
        .collect();

    Scene { nodes, edges }
}

#[cfg(test)]
mod tests {
    use bn_core::model::State;
    use bn_session::{Document, Point};

    use super::*;

    fn two_states() -> Vec<State> {
        vec![State::new("a"), State::new("b")]
    }

    #[test]
    fn scene_reflects_positions_sizes_and_edges() {
        let mut doc = Document::new();
        let a = doc.add_node_at(NodeKind::Chance, Point::new(10.0, 20.0)).unwrap();
        let b = doc.add_node_at(NodeKind::Decision, Point::new(300.0, 20.0)).unwrap();
        let u = doc.add_node_at(NodeKind::Utility, Point::new(600.0, 20.0)).unwrap();
        doc.begin_change();
        doc.net.add_edge(a, b).unwrap();
        doc.net.add_edge(b, u).unwrap();

        let scene = build_scene(&doc, &HashMap::new());
        assert_eq!(scene.nodes.len(), 3);
        assert_eq!(scene.edges.len(), 2);

        let ra = scene.rect_of(a).unwrap();
        assert_eq!((ra.x, ra.y), (10.0, 20.0));
        assert_eq!((ra.w, ra.h), (180.0, 56.0)); // BeliefBars, 2 states
        let ru = scene.rect_of(u).unwrap();
        assert_eq!((ru.w, ru.h), (150.0, 44.0)); // utility

        // a→b is informational (child is a decision node).
        let e_ab = scene.edges.iter().find(|e| e.key == (a, b)).unwrap();
        assert!(e_ab.informational);
        let e_bu = scene.edges.iter().find(|e| e.key == (b, u)).unwrap();
        assert!(!e_bu.informational);

        // Endpoints sit on the node borders, not centers.
        assert!((e_ab.from.x - (10.0 + 180.0)).abs() < 1e-9);
        assert!((e_ab.to.x - 300.0).abs() < 1e-9);
    }

    #[test]
    fn drag_override_wins_over_document_position() {
        let mut doc = Document::new();
        let a = doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        let mut drag = HashMap::new();
        drag.insert(a, (123.0, 456.0));
        let scene = build_scene(&doc, &drag);
        let ra = scene.rect_of(a).unwrap();
        assert_eq!((ra.x, ra.y), (123.0, 456.0));
    }

    #[test]
    fn bounds_unions_all_nodes() {
        let mut doc = Document::new();
        doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        doc.add_node_at(NodeKind::Chance, Point::new(500.0, 300.0)).unwrap();
        let scene = build_scene(&doc, &HashMap::new());
        let b = scene.bounds().unwrap();
        assert_eq!((b.x, b.y), (0.0, 0.0));
        assert_eq!(b.w, 500.0 + 180.0);
        assert_eq!(b.h, 300.0 + 56.0);
    }

    #[test]
    fn title_only_uses_title_or_name() {
        let mut doc = Document::new();
        let a = doc.add_node_at(NodeKind::Chance, Point::new(0.0, 0.0)).unwrap();
        doc.net.set_title(a, "a much longer node title".into());
        doc.visual.get_mut(a).unwrap().display = bn_core::io::DisplayMode::TitleOnly;
        let scene = build_scene(&doc, &HashMap::new());
        let ra = scene.rect_of(a).unwrap();
        assert_eq!(ra.h, 26.0);
        assert!((ra.w - (24.0_f64 * 7.5 + 24.0)).abs() < 1e-9);
        let _ = two_states();
    }
}
