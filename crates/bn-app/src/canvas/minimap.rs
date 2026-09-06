//! Minimap: scene bounds scaled into a fixed box, node rects tinted by
//! kind, the current viewport as a dashed rect; click to center the view.
//! Rewritten from the upstream WorkflowMinimap over measured bounds instead
//! of hardcoded world constants.

use dioxus::prelude::*;

use crate::canvas::controller::VIEWPORT;
use crate::canvas::node::kind_header_color;
use crate::canvas::scene::Scene;
use crate::state::SESSION;

const MINI_W: f64 = 140.0;
const MINI_H: f64 = 100.0;
const PAD: f64 = 6.0;

#[component]
pub fn Minimap(scene: Scene) -> Element {
    let Some(bounds) = scene.bounds() else { return rsx! {} };
    let scale =
        ((MINI_W - 2.0 * PAD) / bounds.w.max(1.0)).min((MINI_H - 2.0 * PAD) / bounds.h.max(1.0));
    let to_mini = move |x: f64, y: f64| -> (f64, f64) {
        (PAD + (x - bounds.x) * scale, PAD + (y - bounds.y) * scale)
    };

    let vp = *VIEWPORT.read();
    let tl = vp.element_to_world(0.0, 0.0);
    let br = vp.element_to_world(vp.size.0, vp.size.1);
    let (vx, vy) = to_mini(tl.x, tl.y);
    let (vw, vh) = ((br.x - tl.x) * scale, (br.y - tl.y) * scale);

    let kinds: Vec<(f64, f64, f64, f64, &'static str)> = {
        let s = SESSION.read();
        scene
            .nodes
            .iter()
            .filter(|n| s.doc.net.contains(n.id))
            .map(|n| {
                let (x, y) = to_mini(n.rect.x, n.rect.y);
                (x, y, n.rect.w * scale, n.rect.h * scale, kind_header_color(s.doc.net.node(n.id).kind))
            })
            .collect()
    };

    rsx! {
        div {
            class: "absolute right-3 bottom-3 overflow-hidden rounded-md border \
                    bg-background/90 shadow-sm",
            style: "width: {MINI_W}px; height: {MINI_H}px;",
            onmousedown: move |ev| ev.stop_propagation(),
            onclick: move |ev| {
                ev.stop_propagation();
                let e = ev.data().element_coordinates();
                let wx = bounds.x + (e.x - PAD) / scale;
                let wy = bounds.y + (e.y - PAD) / scale;
                let mut vp = VIEWPORT.write();
                vp.pan = (
                    vp.size.0 / 2.0 - wx * vp.zoom,
                    vp.size.1 / 2.0 - wy * vp.zoom,
                );
            },
            svg { width: "{MINI_W}", height: "{MINI_H}",
                for (x, y, w, h, color) in kinds {
                    rect {
                        x: "{x:.1}",
                        y: "{y:.1}",
                        width: "{w.max(2.0):.1}",
                        height: "{h.max(2.0):.1}",
                        rx: "1.5",
                        fill: "{color}",
                        stroke: "rgba(0,0,0,0.25)",
                        "stroke-width": "0.5",
                    }
                }
                rect {
                    x: "{vx:.1}",
                    y: "{vy:.1}",
                    width: "{vw:.1}",
                    height: "{vh:.1}",
                    rx: "2",
                    fill: "rgba(100,140,220,0.08)",
                    stroke: "rgba(100,140,220,0.6)",
                    "stroke-width": "1",
                    "stroke-dasharray": "3 2",
                }
            }
        }
    }
}
