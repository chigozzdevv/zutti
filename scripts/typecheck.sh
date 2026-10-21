#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
cargo check --locked --workspace --all-targets
cargo check --locked --manifest-path crates/wallet/ethereum/Cargo.toml --features fulfillment --all-targets
pnpm typecheck:ts
