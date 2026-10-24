#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
cargo test --locked --workspace --all-targets
cargo test --locked --manifest-path crates/boundless/Cargo.toml
cargo test --locked --manifest-path crates/prover/spv/Cargo.toml --features recovery --all-targets
forge test --root contracts
node --test tests/vectors.mjs
cargo test --locked --manifest-path crates/wallet/ethereum/Cargo.toml --features fulfillment,spv-fulfillment
python3 -m unittest discover -s scripts -p 'test_spv_*.py'
python3 -m unittest discover -s crates/prover/spv -p 'test_*.py'
python3 scripts/check-ethereum-layout.py
bash scripts/test-typescript.sh
