#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
cargo build --locked --workspace --all-targets
cargo build --locked --manifest-path crates/boundless/Cargo.toml
forge build --root contracts
cargo build --locked --manifest-path crates/wallet/ethereum/Cargo.toml --features fulfillment
cargo build --locked -p zutti-protocol --features native --bin zutti-core
pnpm codegen
pnpm build:ts
