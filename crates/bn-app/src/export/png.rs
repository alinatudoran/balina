//! SVG → PNG rasterization via resvg (usvg text layout + tiny-skia).

use resvg::tiny_skia;
use resvg::usvg;

/// Rasterize an SVG string at `scale` (2.0 = 2x). Blocking (font loading +
/// rendering) — call inside `spawn_blocking`.
pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1))
        .ok_or_else(|| format!("cannot allocate {w}x{h} image"))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_to_png_bytes() {
        // Text-free so the test does not depend on installed fonts.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="10" height="10" fill="red"/></svg>"#;
        let png = svg_to_png(svg, 2.0).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
    }
}
