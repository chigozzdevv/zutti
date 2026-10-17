#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
cargo fmt --all --check
cargo fmt --manifest-path crates/prover/methods/guest/Cargo.toml --check
cargo fmt --manifest-path crates/boundless/Cargo.toml --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked --manifest-path crates/boundless/Cargo.toml --all-targets -- -D warnings
forge fmt --check contracts/src contracts/test
pnpm exec eslint tests --max-warnings=0
pnpm exec prettier --check package.json pnpm-workspace.yaml tsconfig.base.json eslint.config.js prettier.config.js 'tests/**/*.mjs'
