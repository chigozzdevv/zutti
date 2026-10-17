#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export RISC0_BUILD_LOCKED=1
export RISC0_DEV_MODE=0
mkdir -p .data/proof-evidence
export ZUTTI_PROOF_DIR="$(mktemp -d "$PWD/.data/proof-evidence/run-XXXXXXXX")"
cargo test --locked -p zutti-prover --test history_query -- --ignored --nocapture --test-threads=1
cargo test --locked -p zutti-prover --test preparation -- --ignored --nocapture --test-threads=1
