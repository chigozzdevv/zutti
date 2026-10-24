# SPV proving host

The native host for Zutti's selected Testnet source and conditional-settlement programs. It is an isolated Cargo workspace so current SPV work can run without building the legacy full-validity guests. The executable keeps its existing name, `zutti-spv-prototype`.

Source headers are verified from the fixed network genesis. Payment coverage parses complete blocks in the exact bound interval. Transaction validity relies on miners. Source admission, prepared-payment binding and private-pool settlement use separate programs and exact proof dependencies.

## Build and verify

Run from the repository root. Add `--offline` when dependencies are already cached.

```sh
cargo build --locked --manifest-path crates/prover/spv/Cargo.toml
cargo test --locked --manifest-path crates/prover/spv/Cargo.toml --features recovery --tests
cargo clippy --locked --manifest-path crates/prover/spv/Cargo.toml --features recovery --all-targets -- -D warnings
python3 -m unittest discover -s crates/prover/spv -p 'test_*.py'
```

Use an existing `CARGO_TARGET_DIR` to reuse compiled dependencies. The test suite verifies genuine retained receipts, rejects fake/tampered evidence, checks payment binding and pool conservation, and exercises independent recovery. It does not generate a live conditional swap proof.

## Build guests

Set `CARGO_HOME` to the populated Cargo cache and `SPV` to the built host executable. Each output directory must be new. `--kind` accepts `source`, `admission`, `binding` or `pool`.

```sh
python3 crates/prover/spv/run.py --seconds 600 --rss-mib 4096 \
  --log /tmp/zutti-spv-build.log -- \
  "$SPV" build --kind source --guest-target /tmp/zutti-guest-cache \
  --out /tmp/zutti-source-program
```

The host uses the unchanged guest manifests in `crates/prover/methods`. `run.py` bounds time and sampled process-group memory; it limits child threads and blocks HTTP downloads through child proxy settings. Reaching a limit produces no proof authority. It never submits a marketplace job.

## Source bootstrap

Follow [the bootstrap workflow](../../../docs/spv-bootstrap.md) to export headers from an existing node, initialize a durable store, prepare batches and accept only genuine receipts. `scripts/spv_source.py` resumes those batches and stops on branch disagreement or the supported-height horizon. Preparation is the default; `--prove` explicitly enables local proof generation.

The retained current-image [genesis fixture](fixtures/genesis-coverage/README.md) supports genuine acceptance and restart tests. Its small policy ends at height four and is not a current-chain deployment policy. Historical genesis fixtures cover genuine verification and wrong-program rejection.

## Execution and proof formats

```sh
"$SPV" demo --program /tmp/zutti-source-program/program.elf --out /tmp/zutti-spv-execution
"$SPV" verify --program /tmp/zutti-source-program/program.elf \
  --receipt /absolute/receipt.bin --journal /absolute/journal.bin
```

`demo` executes by default. Continuations without genuine predecessor receipts are explicitly marked as unresolved execution assumptions. Add `--prove` and the exact required predecessor to generate a genuine receipt. Public header proofs may use `--public-header-receipt` for composite output. Binding, coverage and private-pool dependencies retain their required succinct or Groth16 formats even when the Testnet inputs are deliberately public. A composite receipt is not an Ethereum seal.

`--segment-po2` accepts 16 through 22. Smaller segments can reduce peak memory while increasing proof overhead. Time, RSS and cycle guards are independent; raising a guard does not establish throughput or complete proof generation.

## Prepared-payment binding

```sh
cargo build --locked --manifest-path crates/prover/spv/Cargo.toml --example payment-binding
```

The example checks generated public Ironwood payment effects and rejects altered recipient, amount, expiry and anchor fields. Its synthetic source context is not an authenticated checkpoint. The [binding relation](../../../docs/payment-window-binding.md) and [refund review](../../../docs/refund-window-review.md) remain separate from guest execution success.

The `settlement` and `coverage` examples exercise the corresponding journals, proof dependencies, conservation and interval boundaries. Use their `--help` output for explicit genuine-receipt inputs. Execution with unresolved dependencies cannot authorize settlement.

## Boundless and Ethereum

The Testnet demo permits public swap witnesses through the existing Boundless IPFS transport. The prover and anyone obtaining those published inputs can read them. Witness publication does not relax proof verification, ownership checks or fixed outcomes.

New SPV job export, predecessor receipt delivery, marketplace collection and native import still need end-to-end wiring. Ethereum-consumed receipts need genuine Groth16 wrappers checked against the pinned program and exact journal. `src/ethereum.rs` provides export and verification helpers; it is not an admitted deployment.

## Evidence and remaining work

[evidence.json](evidence.json) consolidates the retained program identities, genuine genesis proof/audit, execution samples and bounded failed proof attempts. Execution samples use earlier binding/pool images and are not proofs of the rebuilt programs. No non-genesis current-source receipt or funded swap is retained.

[The delivery plan](../../../docs/testnet-delivery.md) tracks genuine proof composition, current source history, refund review, Sepolia deployment, wallet/SDK execution and funded recovery. Performance for the complete system remains unestablished.
