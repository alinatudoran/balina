#!/usr/bin/env sh
# Build the structure-learning Web Worker wasm module into the app's assets.
# Required before any web build (`dx serve --web`) — the outputs are
# gitignored. Prereqs:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.128 --locked
# (the wasm-bindgen-cli version MUST match the workspace's locked
# wasm-bindgen — skew is a hard runtime failure).
set -e
cd "$(dirname "$0")/.."

cargo build -p bn-worker --release --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --no-typescript \
  --out-dir crates/bn-app/assets/worker --out-name bn_worker \
  target/wasm32-unknown-unknown/release/bn_worker.wasm

echo "Worker built: crates/bn-app/assets/worker/bn_worker.js / bn_worker_bg.wasm"
