//! The canvas shell: measured viewport, world transform, SVG edge layer,
//! gesture routing. Event-wiring skeleton forked from Dioxus/UI's
//! `WorkflowCanvas` (rust-ui/dioxus-ui, MIT), heavily modified: state moved
//! to the session document, floating edges, id-keyed gestures, unbounded
//! world, measured (not hardcoded) viewport.

use std::rc::Rc;

use dioxus::html::geometry::WheelDelta;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::canvas::controller::{
    self, drag_update, rubber_band_world_rect, Gesture, DRAG_HAPPENED, DRAG_POS, FIT_REQUEST,
    GESTURE, VIEWPORT,
};
use crate::canvas::edge::{EdgeLayer, EdgeMarkers};
use crate::canvas::node::BnNode;
use crate::canvas::preview::ConnectionPreview;
use crate::canvas::scene::build_scene;
use crate::state::{
    exec, ContextMenuState, ContextMenuTarget, CONTEXT_MENU, SELECTION, SESSION,
};

/// Default node footprint used to center palette drops (pre-creation, so
/// the real size is unknown; matches the React constants).
pub fn default_node_footprint(kind: bn_core::model::NodeKind) -> (f64, f64) {
    if kind == bn_core::model::NodeKind::Utility { (150.0, 44.0) } else { (180.0, 56.0) }
}

#[component]
pub fn NetworkCanvas() -> Element {
    let mut root_el: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    let scene = use_memo(move || {
        let s = SESSION.read();
        build_scene(&s.doc, &DRAG_POS.read())
    });

    // Fit-view on request (doc load bumps FIT_REQUEST; also first mount).
    // The webview may not have finished layout when this fires (the rect
    // comes back collapsed), so poll until the measurement is sane and
    // stable before fitting.
    use_effect(move || {
        let _req = *FIT_REQUEST.read();
        let Some(el) = root_el.read().clone() else { return };
        spawn(async move {
            let mut last: Option<(f64, f64)> = None;
            for _ in 0..40 {
                let Ok(rect) = el.get_client_rect().await else { return };
                let size = (rect.width(), rect.height());
                if size.1 > 100.0 && last == Some(size) {
                    let bounds = scene.peek().bounds();
                    let mut vp = VIEWPORT.write();
                    vp.origin = (rect.min_x(), rect.min_y());
                    vp.size = size;
                    if let Some(bounds) = bounds {
                        vp.fit(bounds, 0.15);
                    }
                    return;
                }
                last = Some(size);
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        });
    });

    let remeasure = move || {
        let Some(el) = root_el.peek().clone() else { return };
        spawn(async move {
            if let Ok(rect) = el.get_client_rect().await {
                let mut vp = VIEWPORT.write();
                vp.origin = (rect.min_x(), rect.min_y());
                vp.size = (rect.width(), rect.height());
            }
        });
    };

    // Shared by mouseup and by mousemove-with-no-buttons (a release outside
    // the window never delivers mouseup — WebKit also fires spurious
    // mouseleave during drags, so leave events are ignored entirely).
    let finish_gesture = move |_c: (f64, f64)| {
        let g = GESTURE.read().clone();
        match g {
            Gesture::PendingNodeDrag { .. } | Gesture::Pan { .. } => {
                *GESTURE.write() = Gesture::Idle;
            }
            Gesture::DragNodes { .. } => {
                // ONE move_nodes op per drag = one undo step.
                let moves: Vec<(bn_core::model::NodeId, f32, f32)> = DRAG_POS
                    .read()
                    .iter()
                    .map(|(&id, &(x, y))| (id, x as f32, y as f32))
                    .collect();
                *GESTURE.write() = Gesture::Idle;
                if !moves.is_empty() {
                    exec(|s| bn_session::ops::edit::move_nodes(s, &moves));
                }
                DRAG_POS.write().clear();
            }
            Gesture::RubberBand { start_client, cur_client, additive } => {
                *GESTURE.write() = Gesture::Idle;
                let world = rubber_band_world_rect(&VIEWPORT.read(), start_client, cur_client);
                let hits: Vec<bn_core::model::NodeId> = scene
                    .peek()
                    .nodes
                    .iter()
                    .filter(|n| n.rect.intersects(&world))
                    .map(|n| n.id)
                    .collect();
                let mut sel = SELECTION.write();
                if !additive {
                    sel.nodes.clear();
                    sel.edges.clear();
                }
                sel.nodes.extend(hits);
            }
            Gesture::Connect { .. } => controller::cancel_gesture(),
            Gesture::Reconnect { parent, orig_child, .. } => {
                // Dropped on empty canvas → detach.
                controller::cancel_gesture();
                exec(|s| bn_session::ops::edit::remove_edge(s, parent, orig_child));
            }
            // Palette drags start in the toolbar and are handled at the app
            // root (the canvas may never see the mousedown).
            Gesture::PaletteDrag { .. } => {}
            Gesture::Idle => {}
        }
    };

    let vp = *VIEWPORT.read();
    let transform = vp.world_transform();
    let dot_size = 40.0 * vp.zoom;
    let gesture_now = GESTURE.read().clone();
    let cursor = match gesture_now {
        Gesture::Pan { .. } => "grabbing",
        Gesture::RubberBand { .. } => "crosshair",
        Gesture::Connect { .. } | Gesture::Reconnect { .. } => "crosshair",
        _ => "default",
    };
    // Rubber-band overlay rect in element coords.
    let band_rect = match &gesture_now {
        Gesture::RubberBand { start_client, cur_client, .. } => {
            let (ox, oy) = vp.origin;
            let x = start_client.0.min(cur_client.0) - ox;
            let y = start_client.1.min(cur_client.1) - oy;
            let w = (cur_client.0 - start_client.0).abs();
            let h = (cur_client.1 - start_client.1).abs();
            Some((x, y, w, h))
        }
        _ => None,
    };

    rsx! {
        div {
            class: "relative h-full w-full overflow-hidden outline-none select-none",
            style: "cursor: {cursor}; touch-action: none; \
                    background-image: radial-gradient(circle, #c8c8c8 1.5px, transparent 1.5px); \
                    background-size: {dot_size:.2}px {dot_size:.2}px; \
                    background-position: {vp.pan.0:.2}px {vp.pan.1:.2}px;",

            onmounted: move |ev| {
                root_el.set(Some(ev.data()));
            },
            onresize: move |_| remeasure(),

            onmousedown: move |ev| {
                let data = ev.data();
                match data.trigger_button() {
                    Some(MouseButton::Auxiliary) => {
                        let c = data.client_coordinates();
                        let pan_start = VIEWPORT.read().pan;
                        *GESTURE.write() =
                            Gesture::Pan { start_client: (c.x, c.y), pan_start };
                    }
                    Some(MouseButton::Primary) => {
                        if controller::is_connecting() {
                            controller::cancel_gesture();
                            return;
                        }
                        let shift = data.modifiers().contains(Modifiers::SHIFT);
                        if !shift {
                            crate::state::clear_selection();
                        }
                        let c = data.client_coordinates();
                        *GESTURE.write() = Gesture::RubberBand {
                            start_client: (c.x, c.y),
                            cur_client: (c.x, c.y),
                            additive: shift,
                        };
                    }
                    _ => {}
                }
            },

            onmousemove: move |ev| {
                let c = ev.data().client_coordinates();
                let g = GESTURE.read().clone();
                // Button released outside the window: no mouseup will come.
                if !matches!(g, Gesture::Idle | Gesture::PaletteDrag { .. })
                    && ev.data().held_buttons().is_empty()
                {
                    finish_gesture((c.x, c.y));
                    return;
                }
                match g {
                    Gesture::PendingNodeDrag { start_client, starts } => {
                        let dist = ((c.x - start_client.0).powi(2)
                            + (c.y - start_client.1).powi(2))
                        .sqrt();
                        if dist > controller::DRAG_THRESHOLD {
                            *DRAG_HAPPENED.write() = true;
                            let zoom = VIEWPORT.read().zoom;
                            *DRAG_POS.write() =
                                drag_update(&starts, start_client, (c.x, c.y), zoom);
                            *GESTURE.write() = Gesture::DragNodes { start_client, starts };
                        }
                    }
                    Gesture::DragNodes { start_client, starts } => {
                        let zoom = VIEWPORT.read().zoom;
                        *DRAG_POS.write() = drag_update(&starts, start_client, (c.x, c.y), zoom);
                    }
                    Gesture::Pan { start_client, pan_start } => {
                        let mut vp = VIEWPORT.write();
                        vp.pan = (
                            pan_start.0 + c.x - start_client.0,
                            pan_start.1 + c.y - start_client.1,
                        );
                    }
                    Gesture::RubberBand { start_client, additive, .. } => {
                        *GESTURE.write() = Gesture::RubberBand {
                            start_client,
                            cur_client: (c.x, c.y),
                            additive,
                        };
                    }
                    Gesture::Connect { .. } | Gesture::Reconnect { .. } => {
                        // Over empty canvas (node handlers stop propagation):
                        // follow the cursor, clear any hover.
                        let w = VIEWPORT.read().client_to_world(c.x, c.y);
                        match &mut *GESTURE.write() {
                            Gesture::Connect { cursor_world, over, .. }
                            | Gesture::Reconnect { cursor_world, over, .. } => {
                                *cursor_world = w;
                                *over = None;
                            }
                            _ => {}
                        }
                    }
                    Gesture::PaletteDrag { .. } => {} // handled at the app root
                    Gesture::Idle => {}
                }
            },

            onmouseup: move |ev| {
                let c = ev.data().client_coordinates();
                finish_gesture((c.x, c.y));
            },

            onwheel: move |ev| {
                ev.prevent_default();
                let data = ev.data();
                let (dx, dy) = match data.delta() {
                    WheelDelta::Pixels(p) => (p.x, p.y),
                    WheelDelta::Lines(p) => (p.x * 20.0, p.y * 20.0),
                    WheelDelta::Pages(p) => (p.x * 400.0, p.y * 400.0),
                };
                let mods = data.modifiers();
                if mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META) {
                    // Trackpad pinch arrives as ctrl+wheel in WKWebView.
                    let c = data.client_coordinates();
                    let mut vp = VIEWPORT.write();
                    let (ex, ey) = (c.x - vp.origin.0, c.y - vp.origin.1);
                    vp.zoom_at(ex, ey, dy);
                } else {
                    let mut vp = VIEWPORT.write();
                    vp.pan = (vp.pan.0 - dx, vp.pan.1 - dy);
                }
            },

            oncontextmenu: move |ev| {
                ev.prevent_default();
                let c = ev.data().client_coordinates();
                let world = VIEWPORT.read().client_to_world(c.x, c.y);
                *CONTEXT_MENU.write() = Some(ContextMenuState {
                    target: ContextMenuTarget::Pane { world: (world.x, world.y) },
                    client: (c.x, c.y),
                });
            },

            // ── world (transformed, unbounded) ──────────────────────────
            div {
                style: "position: absolute; top: 0; left: 0; width: 0; height: 0; \
                        transform: {transform}; transform-origin: 0 0;",

                svg {
                    style: "position: absolute; top: 0; left: 0; overflow: visible; pointer-events: none;",
                    width: "1",
                    height: "1",
                    EdgeMarkers {}
                    EdgeLayer { edges: scene.read().edges.clone() }
                    ConnectionPreview { scene: scene.read().clone() }
                }

                for n in scene.read().nodes.iter() {
                    BnNode { key: "{n.id:?}", id: n.id, rect: n.rect }
                }
            }

            crate::canvas::minimap::Minimap { scene: scene.read().clone() }

            // ── rubber-band overlay (element space) ─────────────────────
            if let Some((x, y, w, h)) = band_rect {
                div {
                    class: "pointer-events-none absolute rounded-sm border",
                    style: "left:{x:.1}px; top:{y:.1}px; width:{w:.1}px; height:{h:.1}px; \
                            border-color: rgba(100,140,220,0.6); background: rgba(100,140,220,0.1);",
                }
            }
        }
    }
}
