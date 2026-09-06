//! Pure canvas geometry: node sizing, floating-edge clipping, viewport
//! transforms. Ports of the egui `canvas.rs` painters via the React
//! `graph.ts` / `edgeGeometry.ts` (the math comes home to Rust).

use bn_core::io::DisplayMode;
use bn_core::model::NodeKind;

pub const MIN_ZOOM: f64 = 0.15;
pub const MAX_ZOOM: f64 = 4.0;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn center(&self) -> Pt {
        Pt::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && self.x + self.w > o.x && self.y < o.y + o.h && self.y + self.h > o.y
    }

    /// Smallest rect containing both.
    pub fn union(&self, o: &Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// Pixel size of a node, by kind/display (port of `graph.ts nodeSize`, née
/// egui `canvas.rs node_size`). Heights are COMPUTED, never DOM-measured, so
/// edge math stays pure.
pub fn node_size(kind: NodeKind, display: DisplayMode, title: &str, n_states: usize, has_stats_row: bool) -> (f64, f64) {
    if kind == NodeKind::Utility || display == DisplayMode::ExpectedValue {
        return (150.0, 44.0);
    }
    if display == DisplayMode::TitleOnly {
        return ((title.chars().count() as f64 * 7.5 + 24.0).max(90.0), 26.0);
    }
    let extra = if has_stats_row { 14.0 } else { 0.0 };
    (180.0, 20.0 + n_states as f64 * 16.0 + 4.0 + extra)
}

/// Intersection of the segment center→toward with the border of `rect`
/// (rect's center must be `center`).
pub fn clip_to_border(center: Pt, toward: Pt, rect: Rect) -> Pt {
    let dx = toward.x - center.x;
    let dy = toward.y - center.y;
    if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
        return center;
    }
    let tx = if dx.abs() > 1e-6 { rect.w / 2.0 / dx.abs() } else { f64::INFINITY };
    let ty = if dy.abs() > 1e-6 { rect.h / 2.0 / dy.abs() } else { f64::INFINITY };
    let t = tx.min(ty).min(1.0);
    Pt::new(center.x + dx * t, center.y + dy * t)
}

/// Body-to-body floating-edge endpoints: clip the center→center segment to
/// each node's rectangle border.
pub fn edge_endpoints(source: Rect, target: Rect) -> (Pt, Pt) {
    let sc = source.center();
    let tc = target.center();
    (clip_to_border(sc, tc, source), clip_to_border(tc, sc, target))
}

// ---------------------------------------------------------------------------
// Viewport
// ---------------------------------------------------------------------------

/// Pan/zoom + the MEASURED viewport geometry (size and top-left origin in
/// client coordinates — the canvas sits under a toolbar, so origin ≠ 0 and
/// must be re-measured on resize). world→screen: s = w·zoom + pan.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Viewport {
    pub pan: (f64, f64),
    pub zoom: f64,
    pub size: (f64, f64),
    pub origin: (f64, f64),
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { pan: (0.0, 0.0), zoom: 1.0, size: (800.0, 600.0), origin: (0.0, 0.0) }
    }
}

impl Viewport {
    /// Client (window) coordinates → world coordinates.
    pub fn client_to_world(&self, cx: f64, cy: f64) -> Pt {
        self.element_to_world(cx - self.origin.0, cy - self.origin.1)
    }

    /// Element (viewport-relative) coordinates → world coordinates.
    pub fn element_to_world(&self, ex: f64, ey: f64) -> Pt {
        Pt::new((ex - self.pan.0) / self.zoom, (ey - self.pan.1) / self.zoom)
    }

    /// World coordinates → element (viewport-relative) coordinates.
    #[allow(dead_code)] // exercised by the viewport round-trip tests
    pub fn world_to_element(&self, p: Pt) -> Pt {
        Pt::new(p.x * self.zoom + self.pan.0, p.y * self.zoom + self.pan.1)
    }

    /// CSS transform for the world container div.
    pub fn world_transform(&self) -> String {
        format!(
            "translate({:.2}px, {:.2}px) scale({:.4})",
            self.pan.0, self.pan.1, self.zoom
        )
    }

    /// Zoom by an arbitrary scale factor around an element-space point.
    pub fn zoom_at_scale(&mut self, ex: f64, ey: f64, scale: f64) {
        let new_z = (self.zoom * scale).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = new_z / self.zoom;
        self.pan = (ex - (ex - self.pan.0) * ratio, ey - (ey - self.pan.1) * ratio);
        self.zoom = new_z;
    }

    /// Wheel zoom around the cursor (element coords).
    pub fn zoom_at(&mut self, ex: f64, ey: f64, delta_y: f64) {
        let factor = if delta_y < 0.0 { 1.1 } else { 1.0 / 1.1 };
        self.zoom_at_scale(ex, ey, factor);
    }

    /// Center `bounds` in the viewport with fractional padding (15% like
    /// React Flow's default fitView).
    pub fn fit(&mut self, bounds: Rect, padding_frac: f64) {
        if bounds.w <= 0.0 || bounds.h <= 0.0 {
            return;
        }
        let (vw, vh) = self.size;
        let z = ((vw / (bounds.w * (1.0 + 2.0 * padding_frac)))
            .min(vh / (bounds.h * (1.0 + 2.0 * padding_frac))))
        .clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = z;
        self.pan = (
            (vw - bounds.w * z) / 2.0 - bounds.x * z,
            (vh - bounds.h * z) / 2.0 - bounds.y * z,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_size_matches_react_port() {
        use DisplayMode::*;
        use NodeKind::*;
        assert_eq!(node_size(Utility, ExpectedValue, "U", 0, false), (150.0, 44.0));
        assert_eq!(node_size(Chance, ExpectedValue, "X", 2, false), (150.0, 44.0));
        assert_eq!(node_size(Chance, TitleOnly, "ab", 2, false), (90.0, 26.0));
        let (w, _) = node_size(Chance, TitleOnly, "a-rather-long-title", 2, false);
        assert!((w - (19.0 * 7.5 + 24.0)).abs() < 1e-9);
        assert_eq!(node_size(Chance, BeliefBars, "X", 2, false), (180.0, 56.0));
        assert_eq!(node_size(Decision, BeliefBars, "X", 3, false), (180.0, 72.0));
        assert_eq!(node_size(Chance, BeliefBars, "X", 2, true), (180.0, 70.0));
    }

    #[test]
    fn clip_to_border_hits_the_rect_edge() {
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let c = rect.center();
        // Straight right: exits at x=100, y=25.
        let p = clip_to_border(c, Pt::new(200.0, 25.0), rect);
        assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 25.0).abs() < 1e-9);
        // Straight down: exits at y=50.
        let p = clip_to_border(c, Pt::new(50.0, 500.0), rect);
        assert!((p.y - 50.0).abs() < 1e-9 && (p.x - 50.0).abs() < 1e-9);
        // Target inside the rect: t clamps at 1 → returns the target.
        let p = clip_to_border(c, Pt::new(60.0, 30.0), rect);
        assert!((p.x - 60.0).abs() < 1e-9 && (p.y - 30.0).abs() < 1e-9);
        // Degenerate: same point.
        let p = clip_to_border(c, c, rect);
        assert_eq!(p, c);
    }

    #[test]
    fn edge_endpoints_clip_both_sides() {
        let a = Rect::new(0.0, 0.0, 100.0, 50.0);
        let b = Rect::new(300.0, 0.0, 100.0, 50.0);
        let (from, to) = edge_endpoints(a, b);
        assert!((from.x - 100.0).abs() < 1e-9, "leaves a's right border");
        assert!((to.x - 300.0).abs() < 1e-9, "enters b's left border");
        assert!((from.y - 25.0).abs() < 1e-9 && (to.y - 25.0).abs() < 1e-9);
    }

    #[test]
    fn viewport_roundtrip_and_zoom_anchor() {
        let mut vp = Viewport {
            pan: (30.0, -12.0),
            zoom: 1.7,
            size: (900.0, 500.0),
            origin: (0.0, 38.0),
        };
        let w = vp.client_to_world(400.0, 300.0);
        let back = vp.world_to_element(w);
        assert!((back.x - 400.0).abs() < 1e-9);
        assert!((back.y - (300.0 - 38.0)).abs() < 1e-9);

        // The world point under the cursor is invariant across a zoom.
        let anchor_el = (200.0, 150.0);
        let before = vp.element_to_world(anchor_el.0, anchor_el.1);
        vp.zoom_at(anchor_el.0, anchor_el.1, -1.0);
        let after = vp.element_to_world(anchor_el.0, anchor_el.1);
        assert!((before.x - after.x).abs() < 1e-9 && (before.y - after.y).abs() < 1e-9);
    }

    #[test]
    fn zoom_clamps() {
        let mut vp = Viewport::default();
        for _ in 0..100 {
            vp.zoom_at(0.0, 0.0, 1.0); // zoom out
        }
        assert!((vp.zoom - MIN_ZOOM).abs() < 1e-9);
        for _ in 0..200 {
            vp.zoom_at(0.0, 0.0, -1.0); // zoom in
        }
        assert!((vp.zoom - MAX_ZOOM).abs() < 1e-9);
    }

    #[test]
    fn fit_centers_bounds() {
        let mut vp = Viewport { size: (800.0, 600.0), ..Default::default() };
        let bounds = Rect::new(100.0, 100.0, 400.0, 200.0);
        vp.fit(bounds, 0.15);
        // Bounds center maps to viewport center.
        let c = vp.world_to_element(bounds.center());
        assert!((c.x - 400.0).abs() < 1e-6);
        assert!((c.y - 300.0).abs() < 1e-6);
    }
}
