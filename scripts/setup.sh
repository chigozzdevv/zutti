#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo rustc pnpm node forge r0vm rzup; do
  command -v "$tool" >/dev/null || { echo "Missing required tool: $tool" >&2; exit 1; }
done
pnpm install --frozen-lockfile
cargo fetch --locked
cargo fetch --locked --manifest-path crates/prover/methods/guest/Cargo.toml
cargo fetch --locked --manifest-path crates/boundless/Cargo.toml
cargo fetch --locked --manifest-path crates/wallet/ethereum/Cargo.toml
