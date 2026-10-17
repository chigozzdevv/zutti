#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .data
run_dir="$(mktemp -d .data/boundless-XXXXXXXX)"
public_dir="$PWD/$run_dir/preparation"
cargo run --locked -p zutti-prover --example public-preparation -- "$public_dir"
bash scripts/boundless.sh submit --job "$public_dir/job.json" --request "$public_dir/request.json"
bash scripts/boundless.sh collect --request "$public_dir/request.json" --out "$public_dir/result.json"
bash scripts/boundless.sh evidence --request "$public_dir/request.json" --out "$public_dir/evidence.json"
cargo run --locked -p zutti-prover --bin private-prover -- import-boundless \
  --result "$public_dir/result.json" --statement "$public_dir/statement.bin" --out "$public_dir/preparation.proof"
ZUTTI_PUBLIC_JOB_DIR="$public_dir" forge test --root contracts --match-test testRealBoundlessPreparationAndTampering -j 1
