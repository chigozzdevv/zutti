#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --locked -p zutti-protocol --features native --bin zutti-core
task_target="${CARGO_TARGET_DIR:-$PWD/target}"
if [[ "$task_target" != /* ]]; then task_target="$PWD/$task_target"; fi
export ZUTTI_CORE_BINARY="${ZUTTI_CORE_BINARY:-$task_target/debug/zutti-core}"
pnpm build:ts
task_tests=(packages/*/test/*.test.mjs)
if [[ -d server/test ]]; then task_tests+=(server/test/*.test.mjs); fi
node --test "${task_tests[@]}"
