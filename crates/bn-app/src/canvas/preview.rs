//! Live connection preview: green over a valid target, red over an invalid
//! one, gray over empty canvas (exact egui palette). Straight line +
//! arrowhead polygon, rendered in the world-space SVG layer with
//! pointer-events off so it never blocks node hover.

use dioxus::prelude::*;

use crate::canvas::controller::{Gesture, GESTURE};
use crate::canvas::geometry::{clip_to_border, Pt};
use crate::canvas::scene::Scene;

#[component]
pub fn ConnectionPreview(scene: Scene) -> Element {
    let g = GESTURE.read();
    let (source, cursor, over) = match &*g {
        Gesture::Connect { from, cursor_world, over } => (*from, *cursor_world, over.clone()),
        Gesture::Reconnect { parent, cursor_world, over, .. } => {
            (*parent, *cursor_world, over.clone())
        }
        _ => return rsx! {},
    };
    drop(g);

    let Some(rect) = scene.rect_of(source) else { return rsx! {} };
    let from = clip_to_border(rect.center(), cursor, rect);
    // Anchor the tip to the hovered node's border for a stable look.
    let to: Pt = over
        .as_ref()
        .and_then(|h| scene.rect_of(h.node))
        .map(|r| clip_to_border(r.center(), from, r))
        .unwrap_or(cursor);

    let color = match &over {
        Some(h) if h.valid => "rgb(80,200,90)",
        Some(_) => "rgb(230,80,80)",
        None => "rgb(150,150,150)",
    };

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (ux, uy) = (dx / len, dy / len);
    let (ah, aw) = (10.0, 5.0);
    let bx = to.x - ux * ah;
    let by = to.y - uy * ah;
    let points = format!(
        "{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
        to.x,
        to.y,
        bx - uy * aw,
        by + ux * aw,
        bx + uy * aw,
        by - ux * aw
    );

    rsx! {
        g { style: "pointer-events: none;",
            path {
                d: "M {from.x:.1} {from.y:.1} L {to.x:.1} {to.y:.1}",
                fill: "none",
                stroke: "{color}",
                "stroke-width": "2",
            }
            polygon { points: "{points}", fill: "{color}" }
        }
    }
}
