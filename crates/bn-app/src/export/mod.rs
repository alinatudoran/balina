//! Diagram export: File > Export as SVG… / PNG…. Read-only — no session
//! ops involved; the SVG is generated synchronously from the session before
//! any await, then written off the UI thread.

pub mod png;
pub mod svg;

use std::path::PathBuf;

use dioxus::prelude::*;

use crate::state::{log_message, SESSION};

pub fn export_svg() {
    export_with("SVG image", "svg", |svg| Ok(svg.into_bytes()));
}

pub fn export_png() {
    export_with("PNG image", "png", |svg| png::svg_to_png(&svg, 2.0));
}

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
