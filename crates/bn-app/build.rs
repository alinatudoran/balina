//! The web build embeds the bn-worker artifacts (include_bytes!). They are
//! gitignored and produced by scripts/build-worker.sh; create empty
//! placeholders so plain `cargo check/build` works before that script has
//! run — the app detects empty bytes at runtime and fails with a clear
//! message instead.

use std::path::Path;

fn main() {
    for f in ["assets/worker/bn_worker.js", "assets/worker/bn_worker_bg.wasm"] {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(f);
        if !p.exists() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(&p, []);
        }
        println!("cargo::rerun-if-changed={f}");
    }
}
