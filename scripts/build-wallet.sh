#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bindgen="${WASM_BINDGEN:-wasm-bindgen}"
command -v "$bindgen" >/dev/null
cargo build --locked --manifest-path crates/wallet/browser/Cargo.toml --release --target wasm32-unknown-unknown
target="${CARGO_TARGET_DIR:-crates/wallet/browser/target}"
"$bindgen" "$target/wasm32-unknown-unknown/release/zutti_browser_wallet.wasm" \
  --target web --omit-default-module-path --out-name wallet --out-dir packages/snap/generated
node --experimental-strip-types packages/snap/build.mjs
