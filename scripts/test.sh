#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
cargo test --locked --workspace --all-targets
cargo test --locked --manifest-path crates/boundless/Cargo.toml
forge test --root contracts
node --test tests/vectors.mjs
