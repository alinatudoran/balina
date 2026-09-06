//! Floating edges: straight lines clipped to node borders, arrowheads,
//! dashed for informational (decision-child) links. A wide translucent
//! understroke shows selection, matching the egui canvas. Hit-testing uses
//! an invisible fat stroke; a grab circle at the target end starts a
//! reconnect when the edge is selected.

use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::canvas::controller::{Gesture, GESTURE, VIEWPORT};
use crate::canvas::scene::EdgeGeom;
use crate::state::{ContextMenuState, ContextMenuTarget, CONTEXT_MENU, SELECTION};

pub const EDGE_STROKE: &str = "#555";
pub const EDGE_SELECTED: &str = "rgb(100,140,220)";

/// SVG `defs` with the two arrowhead markers (markers cannot inherit stroke
/// color, so selected edges get their own).
#[component]
pub fn EdgeMarkers() -> Element {
    rsx! {
        defs {
            for (mid, color) in [("bn-arrow", EDGE_STROKE), ("bn-arrow-sel", EDGE_SELECTED)] {
                marker {
                    id: "{mid}",
                    "markerWidth": "9",
                    "markerHeight": "7",
                    "refX": "8",
                    "refY": "3",
                    orient: "auto",
                    "markerUnits": "userSpaceOnUse",
                    path { d: "M0,0 L0,6 L8,3 z", fill: "{color}" }
                }
            }
        }
    }
}

#[component]
pub fn EdgeLayer(edges: Vec<EdgeGeom>) -> Element {
    rsx! {
        for e in edges.into_iter() {
            EdgePath { edge: e }
        }
    }
}

#[component]
fn EdgePath(edge: EdgeGeom) -> Element {
    let (parent, child) = edge.key;
    let selected = SELECTION.read().edges.contains(&edge.key);
    let d = format!(
        "M {:.1} {:.1} L {:.1} {:.1}",
        edge.from.x, edge.from.y, edge.to.x, edge.to.y
    );
    let stroke = if selected { EDGE_SELECTED } else { EDGE_STROKE };
    let width = if selected { 2.5 } else { 1.5 };
    let marker = if selected { "url(#bn-arrow-sel)" } else { "url(#bn-arrow)" };
    let dash = if edge.informational { "8 5" } else { "none" };
    let (tx, ty) = (edge.to.x, edge.to.y);

    rsx! {
        g {
            if selected {
                path {
                    d: "{d}",
                    fill: "none",
                    stroke: "rgba(100,140,220,0.35)",
                    "stroke-width": "7",
                }
            }
            path {
                d: "{d}",
                fill: "none",
                stroke: "{stroke}",
                "stroke-width": "{width}",
                "stroke-dasharray": "{dash}",
                "marker-end": "{marker}",
            }
            // Invisible fat path for hover/click/context-menu.
            path {
                d: "{d}",
                fill: "none",
                stroke: "transparent",
                "stroke-width": "12",
                style: "pointer-events: stroke; cursor: pointer;",
                onmousedown: move |ev| {
                    if ev.data().trigger_button() == Some(MouseButton::Primary) {
                        ev.stop_propagation();
                    }
                },
                onclick: move |ev| {
                    ev.stop_propagation();
                    let mut sel = SELECTION.write();
                    if !sel.edges.remove(&(parent, child)) {
                        sel.nodes.clear();
                        sel.edges.clear();
                        sel.edges.insert((parent, child));
                    }
                },
                oncontextmenu: move |ev| {
                    ev.prevent_default();
                    ev.stop_propagation();
                    let c = ev.data().client_coordinates();
                    *CONTEXT_MENU.write() = Some(ContextMenuState {
                        target: ContextMenuTarget::Edge(parent, child),
                        client: (c.x, c.y),
                    });
                },
            }
            // Reconnect grab handle at the target end (selected edges only).
            if selected {
                circle {
                    cx: "{tx:.1}",
                    cy: "{ty:.1}",
                    r: "8",
                    fill: "rgba(100,140,220,0.15)",
                    stroke: "{EDGE_SELECTED}",
                    "stroke-width": "1",
                    style: "pointer-events: all; cursor: crosshair;",
                    onmousedown: move |ev| {
                        if ev.data().trigger_button() != Some(MouseButton::Primary) {
                            return;
                        }
                        ev.stop_propagation();
                        let c = ev.data().client_coordinates();
                        let cursor_world = VIEWPORT.read().client_to_world(c.x, c.y);
                        *GESTURE.write() = Gesture::Reconnect {
                            parent,
                            orig_child: child,
                            cursor_world,
                            over: None,
                        };
                    },
                }
            }
        }
    }
}
