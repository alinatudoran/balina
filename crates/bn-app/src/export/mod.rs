//! Diagram export: File > Export as SVG… / PNG…. Read-only — no session
//! ops involved; the SVG is generated synchronously from the session.
//! Desktop rasterizes PNG with resvg and writes via a save dialog; web
//! rasterizes with the browser's own canvas and downloads.

#[cfg(not(target_arch = "wasm32"))]
pub mod png;
pub mod svg;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use dioxus::prelude::*;

use crate::state::{log_message, SESSION};

pub fn export_svg() {
    #[cfg(not(target_arch = "wasm32"))]
    export_with("SVG image", "svg", |svg| Ok(svg.into_bytes()));
    #[cfg(target_arch = "wasm32")]
    export_web(false);
}

pub fn export_png() {
    #[cfg(not(target_arch = "wasm32"))]
    export_with("PNG image", "png", |svg| png::svg_to_png(&svg, 2.0));
    #[cfg(target_arch = "wasm32")]
    export_web(true);
}

#[cfg(target_arch = "wasm32")]
fn export_web(png: bool) {
    let (svg, name) = {
        let s = SESSION.read();
        (svg::network_svg(&s.doc, &s.bridge), s.doc.net.name.clone())
    };
    let Some(svg) = svg else {
        log_message("Nothing to export: the network has no nodes.");
        return;
    };
    if png {
        crate::platform::download_png_from_svg(&format!("{name}.png"), &svg, 2.0);
        log_message(format!("Exported {name}.png (downloaded)."));
    } else {
        crate::platform::download(&format!("{name}.svg"), "image/svg+xml", svg.as_bytes());
        log_message(format!("Exported {name}.svg (downloaded)."));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn export_with(
    filter_name: &'static str,
    ext: &'static str,
    encode: impl FnOnce(String) -> Result<Vec<u8>, String> + Send + 'static,
) {
    spawn(async move {
        let (svg, name) = {
            let s = SESSION.read();
            (svg::network_svg(&s.doc, &s.bridge), s.doc.net.name.clone())
        };
        let Some(svg) = svg else {
            log_message("Nothing to export: the network has no nodes.");
            return;
        };
        let picked = rfd::AsyncFileDialog::new()
            .add_filter(filter_name, &[ext])
            .set_file_name(format!("{name}.{ext}"))
            .save_file()
            .await;
        let Some(fh) = picked else { return };
        let path: PathBuf = fh.path().to_path_buf();
        let written = tokio::task::spawn_blocking(move || {
            let bytes = encode(svg)?;
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Ok::<PathBuf, String>(path)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match written {
            Ok(p) => log_message(format!("Exported {}.", p.display())),
            Err(e) => log_message(format!("Export failed: {e}")),
        }
    });
}
