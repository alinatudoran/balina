//! Node widget: kind-tinted header, belief-bar rows (clickable to set
//! hard evidence), TitleOnly / ExpectedValue modes. Port of the React
//! `BnNode.tsx` (itself a port of the egui canvas painter).

use bn_core::inference::Finding;
use bn_core::io::DisplayMode;
use bn_core::model::{NodeId, NodeKind};
use dioxus::html::input_data::MouseButton;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;

use crate::canvas::controller::{self, Gesture, Hover, DRAG_HAPPENED, GESTURE, VIEWPORT};
use crate::canvas::geometry::Rect;
use crate::logic::format::{expected_value, pct, variance};
use crate::state::{
    exec, open_dialog, ContextMenuState, ContextMenuTarget, DialogDesc, CONTEXT_MENU, SELECTION,
    SESSION,
};

pub const BAR_ORANGE: &str = "rgb(220,150,60)";
pub const BAR_BLUE: &str = "rgb(90,140,220)";
pub const BAR_FINDING: &str = "rgb(110,110,110)";

pub fn kind_body_color(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Chance => "rgb(255,248,220)",
        NodeKind::Decision => "rgb(219,233,255)",
        NodeKind::Utility => "rgb(255,226,226)",
    }
}

pub fn kind_header_color(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Chance => "rgb(240,222,160)",
        NodeKind::Decision => "rgb(173,205,250)",
        NodeKind::Utility => "rgb(247,182,182)",
    }
}

/// One belief row's display: value text, bar fraction (0..=1) and bar color.
/// Decision nodes show EU per choice normalized to the [min, max] EU span;
/// everything else shows the belief as a percentage. Shared by the canvas
/// `BeliefRow` and the SVG exporter so they cannot drift.
pub fn belief_row_display(
    kind: NodeKind,
    s: usize,
    beliefs: Option<&[f64]>,
    decision_eu: Option<&[Option<f64>]>,
    finding_here: bool,
) -> (String, f64, &'static str) {
    if kind == NodeKind::Decision {
        let eu = decision_eu.and_then(|v| v.get(s).copied().flatten());
        if let (Some(eu), Some(eus)) = (eu, decision_eu) {
            let vals: Vec<f64> = eus.iter().copied().flatten().collect();
            let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let frac = if hi > lo { (eu - lo) / (hi - lo) } else { 1.0 };
            (format!("{eu:.2}"), frac, BAR_BLUE)
        } else if let Some(b) = beliefs {
            (pct(b[s]), b[s], BAR_BLUE)
        } else {
            ("--".into(), 0.0, "gray")
        }
    } else if let Some(b) = beliefs {
        (pct(b[s]), b[s], if finding_here { BAR_FINDING } else { BAR_ORANGE })
    } else {
        ("--".into(), 0.0, "gray")
    }
}

/// Border precedence: selection > link-target > finding > default.
fn border_style(selected: bool, link_target: Option<bool>, has_finding: bool) -> (String, f64) {
    if selected {
        return ("rgb(100,140,220)".into(), 2.5);
    }
    if let Some(valid) = link_target {
        return (if valid { "rgb(80,200,90)" } else { "rgb(230,80,80)" }.into(), 2.5);
    }
    if has_finding {
        return ("rgb(90,90,90)".into(), 2.5);
    }
    ("rgb(140,140,140)".into(), 1.2)
}

/// Snapshot of everything a node renders, cloned out of the session in one
/// read so no borrow is held inside event handlers.
#[derive(Clone, PartialEq)]
struct NodeData {
    name: String,
    title: String,
    kind: NodeKind,
    display: DisplayMode,
    state_names: Vec<String>,
    state_values: Vec<Option<f64>>,
    color: Option<[u8; 3]>,
    finding_state: Option<usize>,
    has_finding: bool,
    beliefs: Option<Vec<f64>>,
    decision_eu: Option<Vec<Option<f64>>>,
    utility_ev: Option<f64>,
}

fn read_node_data(id: NodeId) -> Option<NodeData> {
    let s = SESSION.read();
    if !s.doc.net.contains(id) {
        return None;
    }
    let n = s.doc.net.node(id);
    let v = s.doc.visual.get(id);
    let finding = s.doc.evidence.get(id);
    Some(NodeData {
        name: n.name.clone(),
        title: n.title.clone(),
        kind: n.kind,
        display: v.map(|v| v.display).unwrap_or_default(),
        state_names: n.states.iter().map(|st| st.name.clone()).collect(),
        state_values: n.states.iter().map(|st| st.value).collect(),
        color: v.and_then(|v| v.color),
        finding_state: match finding {
            Some(Finding::Hard(st)) => Some(*st),
            _ => None,
        },
        has_finding: finding.is_some(),
        beliefs: s.bridge.beliefs.get(id).cloned(),
        decision_eu: s.bridge.decision_eu.get(id).cloned(),
        utility_ev: s.bridge.utility_ev.get(id).copied().filter(|v| v.is_finite()),
    })
}

/// World positions of the nodes a drag would move: the clicked node plus the
/// rest of the selection (if the clicked node is part of it).
fn drag_starts(clicked: NodeId) -> Vec<(NodeId, f64, f64)> {
    let s = SESSION.read();
    let sel = SELECTION.read();
    let ids: Vec<NodeId> = if sel.nodes.contains(&clicked) {
        sel.nodes.iter().copied().collect()
    } else {
        vec![clicked]
    };
    ids.into_iter()
        .filter_map(|id| {
            s.doc.visual.get(id).map(|v| (id, v.pos.x as f64, v.pos.y as f64))
        })
        .collect()
}

#[component]
pub fn BnNode(id: NodeId, rect: Rect) -> Element {
    let Some(d) = read_node_data(id) else { return rsx! {} };

    let selected = SELECTION.read().nodes.contains(&id);
    // Live link feedback while a connection drag hovers this node.
    let link_target: Option<bool> = match &*GESTURE.read() {
        Gesture::Connect { from, over: Some(Hover { node, valid }), .. }
            if *node == id && *from != id =>
        {
            Some(*valid)
        }
        Gesture::Reconnect { parent, over: Some(Hover { node, valid }), .. }
            if *node == id && *parent != id =>
        {
            Some(*valid)
        }
        _ => None,
    };

    let (border_color, border_width) = border_style(selected, link_target, d.has_finding);
    let body = kind_body_color(d.kind);
    let header_bg = d
        .color
        .map(|c| format!("rgb({},{},{})", c[0], c[1], c[2]))
        .unwrap_or_else(|| kind_header_color(d.kind).to_string());
    let show_bars = d.display == DisplayMode::BeliefBars && d.kind != NodeKind::Utility;
    let title_only = d.display == DisplayMode::TitleOnly && d.kind != NodeKind::Utility;
    let base_title = if d.title.is_empty() { d.name.clone() } else { d.title.clone() };
    let title = format!("{base_title}{}", if d.has_finding { " ⏺" } else { "" });

    let ev_text = if d.kind == NodeKind::Utility {
        match d.utility_ev {
            Some(v) => format!("EU = {v:.2}"),
            None => "EU = --".into(),
        }
    } else {
        let states: Vec<bn_core::model::State> = d
            .state_names
            .iter()
            .zip(&d.state_values)
            .map(|(n, v)| bn_core::model::State { name: n.clone(), value: *v })
            .collect();
        match expected_value(&states, d.beliefs.as_deref()) {
            Some(v) => format!("E = {v:.3}"),
            None => "E = --".into(),
        }
    };

    let sigma_text: Option<String> = if show_bars && d.kind == NodeKind::Chance {
        let states: Vec<bn_core::model::State> = d
            .state_names
            .iter()
            .zip(&d.state_values)
            .map(|(n, v)| bn_core::model::State { name: n.clone(), value: *v })
            .collect();
        let beliefs = d.beliefs.as_deref();
        variance(&states, beliefs).map(|var| {
            let ev = expected_value(&states, beliefs).unwrap_or(f64::NAN);
            format!("E = {ev:.3}  σ = {:.3}", var.sqrt())
        })
    } else {
        None
    };

    let n_states = d.state_names.len();
    let show_handles = d.kind != NodeKind::Utility;
    let d_rows = d.clone();

    rsx! {
        div {
            class: "group absolute rounded-[5px]",
            style: "left: {rect.x:.1}px; top: {rect.y:.1}px; width: {rect.w:.0}px; height: {rect.h:.0}px; \
                    background: {body}; border: {border_width}px solid {border_color}; cursor: grab;",

            onmousedown: move |ev| {
                if ev.data().trigger_button() != Some(MouseButton::Primary) {
                    return; // right/middle: pan or context menu, handled by the canvas
                }
                ev.stop_propagation();
                if controller::is_connecting() {
                    return; // mouseup on this node finishes the connect
                }
                *DRAG_HAPPENED.write() = false;
                let shift = ev.data().modifiers().contains(Modifiers::SHIFT);
                if shift {
                    let mut sel = SELECTION.write();
                    if !sel.nodes.remove(&id) {
                        sel.nodes.insert(id);
                    }
                } else if !SELECTION.read().nodes.contains(&id) {
                    let mut sel = SELECTION.write();
                    sel.nodes.clear();
                    sel.edges.clear();
                    sel.nodes.insert(id);
                }
                let c = ev.data().client_coordinates();
                let starts = drag_starts(id);
                *GESTURE.write() =
                    Gesture::PendingNodeDrag { start_client: (c.x, c.y), starts };
            },

            onmouseup: move |ev| {
                let gesture = GESTURE.read().clone();
                match gesture {
                    Gesture::Connect { from, .. } => {
                        ev.stop_propagation();
                        controller::cancel_gesture();
                        if from != id && controller::check_link_now(from, id, None).ok {
                            exec(|s| bn_session::ops::edit::add_edge(s, from, id));
                        }
                    }
                    Gesture::Reconnect { parent, orig_child, .. } => {
                        ev.stop_propagation();
                        controller::cancel_gesture();
                        if id == orig_child || id == parent {
                            return; // dropped back where it was
                        }
                        let check = controller::check_link_now(parent, id, Some(orig_child));
                        if check.ok {
                            exec(|s| bn_session::ops::edit::move_edge(s, parent, orig_child, id));
                        } else if let Some(reason) = check.reason {
                            crate::state::log_message(format!("Cannot move link: {reason}"));
                        }
                    }
                    _ => {}
                }
            },

            onmousemove: move |ev| {
                // Only relevant while a connect/reconnect is in flight.
                let snapshot = match &*GESTURE.read() {
                    Gesture::Connect { from, .. } => Some((*from, None)),
                    Gesture::Reconnect { parent, orig_child, .. } => {
                        Some((*parent, Some(*orig_child)))
                    }
                    _ => None,
                };
                let Some((src, orig)) = snapshot else { return };
                // Keep the canvas handler from clearing the hover we set.
                ev.stop_propagation();
                let c = ev.data().client_coordinates();
                let w = VIEWPORT.read().client_to_world(c.x, c.y);
                let hover = (src != id).then(|| Hover {
                    node: id,
                    valid: controller::check_link_now(src, id, orig).ok,
                });
                match &mut *GESTURE.write() {
                    Gesture::Connect { cursor_world, over, .. }
                    | Gesture::Reconnect { cursor_world, over, .. } => {
                        *cursor_world = w;
                        *over = hover;
                    }
                    _ => {}
                }
            },

            onmouseleave: move |_| {
                let mut g = GESTURE.write();
                match &mut *g {
                    Gesture::Connect { over, .. } | Gesture::Reconnect { over, .. }
                        if over.as_ref().is_some_and(|h| h.node == id) => {
                            *over = None;
                        }
                    _ => {}
                }
            },

            ondoubleclick: move |ev| {
                ev.stop_propagation();
                open_dialog(DialogDesc::NodeProperties { node: id });
            },

            oncontextmenu: move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                let c = ev.data().client_coordinates();
                *CONTEXT_MENU.write() = Some(ContextMenuState {
                    target: ContextMenuTarget::Node(id),
                    client: (c.x, c.y),
                });
            },

            // ── content ─────────────────────────────────────────────────
            if show_bars {
                div {
                    class: "flex h-5 items-center rounded-t-[4px] px-1.5",
                    style: "background: {header_bg};",
                    span { class: "truncate text-xs font-medium text-neutral-900", "{title}" }
                }
                div { class: "py-0.5",
                    for s in 0..n_states {
                        BeliefRow { id, s, d: d_rows.clone() }
                    }
                }
                if let Some(ref st) = sigma_text {
                    div {
                        class: "border-t border-neutral-200 px-[5px] py-[1px] font-mono text-[9.5px] leading-none text-neutral-600",
                        "{st}"
                    }
                }
            } else if title_only {
                div { class: "flex h-full items-center justify-center px-2",
                    span { class: "truncate text-xs font-medium text-neutral-900", "{title}" }
                }
            } else {
                div { class: "flex h-full flex-col items-center justify-between py-1",
                    span { class: "max-w-full truncate px-2 text-xs font-medium text-neutral-900",
                        "{title}"
                    }
                    span { class: "font-mono text-[11px] text-neutral-800", "{ev_text}" }
                }
            }

            // ── link anchors: 4 side midpoints, visible on hover (not on
            //    utility — utility nodes cannot be parents) ───────────────
            if show_handles {
                for (hx, hy) in [(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)] {
                    div {
                        class: "absolute h-[11px] w-[11px] rounded-full border border-neutral-500 bg-white \
                                opacity-0 transition-opacity group-hover:opacity-100",
                        style: "left: {hx}%; top: {hy}%; transform: translate(-50%, -50%); \
                                cursor: crosshair; z-index: 10;",
                        onmousedown: move |ev| {
                            if ev.data().trigger_button() != Some(MouseButton::Primary) {
                                return;
                            }
                            ev.stop_propagation();
                            let c = ev.data().client_coordinates();
                            let cursor_world = VIEWPORT.read().client_to_world(c.x, c.y);
                            *GESTURE.write() =
                                Gesture::Connect { from: id, cursor_world, over: None };
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn BeliefRow(id: NodeId, s: usize, d: NodeData) -> Element {
    let state_name = d.state_names[s].clone();
    let finding_here = d.finding_state == Some(s);

    let (val_text, frac, bar_color) = belief_row_display(
        d.kind,
        s,
        d.beliefs.as_deref(),
        d.decision_eu.as_deref(),
        finding_here,
    );

    let bar_w = (frac.min(1.0) * 100.0).max(0.0);
    let tip = if finding_here {
        format!("Click to retract the finding {}={state_name}", d.name)
    } else {
        format!("Click to enter the finding {}={state_name}", d.name)
    };

    rsx! {
        div {
            class: "grid h-4 cursor-pointer items-center gap-1 px-[5px] hover:bg-black/5",
            style: "grid-template-columns: 47px 40px 1fr;",
            title: "{tip}",
            onclick: move |ev| {
                ev.stop_propagation();
                if !*DRAG_HAPPENED.read() {
                    exec(|sess| bn_session::ops::evidence::toggle_finding(sess, id, s));
                }
            },
            span { class: "truncate text-[10.5px] leading-none text-neutral-800", "{state_name}" }
            span { class: "whitespace-pre text-right font-mono text-[10px] leading-none text-neutral-800",
                "{val_text}"
            }
            div { class: "relative h-[10px] border border-neutral-300 bg-neutral-100",
                if frac > 0.0 {
                    div {
                        class: "absolute inset-y-0 left-0",
                        style: "width: {bar_w:.2}%; background: {bar_color};",
                    }
                }
            }
        }
    }
}
