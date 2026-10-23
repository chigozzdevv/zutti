# Public source-validation measurements

These diagnostics execute Zutti's transaction authorization and commitment-tree code inside RISC Zero 3.0.5. They generate no receipts, purchase no compute and confer no source or settlement authority.

The seven fixtures are raw public Zcash Testnet blocks. Their heights, block hashes and SHA-256 digests are in `fixtures/manifest.json`. They cover Sapling, the original Orchard circuit, its NU6.2 replacement and Ironwood's NU6.3 circuit. The selected transactions require no external transparent prevouts. The native runner checks each authorization before executing the guest.

## Measure authorization phases

Install the RISC Zero Rust toolchain and `r0vm`, then build the standalone host:

```sh
cargo build --locked --manifest-path benchmarks/validation/host/Cargo.toml
```

Run the three suites with distinct output directories:

```sh
benchmarks/validation/host/target/debug/batch \
  --suite orchard-original --out /tmp/zutti-original

benchmarks/validation/host/target/debug/batch \
  --suite orchard-fixed --out /tmp/zutti-fixed \
  --program /tmp/zutti-original/program.elf

benchmarks/validation/host/target/debug/batch \
  --suite ironwood --out /tmp/zutti-ironwood \
  --program /tmp/zutti-original/program.elf
```

Set `CARGO_HOME` explicitly before the first run. Guest builds use locked, offline dependencies and one compiler worker. If `CARGO_TARGET_DIR` is configured, use its host binary path instead of the default above. All suites must use the same guest program for the estimator to accept them.

Each suite checks a smaller and larger bundle twice in one VM session. The first verification includes initial public-key and arithmetic setup. Subsequent checks reuse the production verifier's `OnceLock` caches. Each call still verifies the full authorization. Repeated inputs measure work; they do not represent distinct transactions.

The guest records parsing and verification phase cycles. Those counters are host-supplied diagnostic values, not cryptographically authenticated performance claims. The runner binds the journal to its exact input, checks the number and bounds of measurements, and records total user cycles separately from padded segment cycles. Neither execution time nor cycles establish GPU proof latency or marketplace fees.

## Trees and rejection checks

`run.py` verifies fixture digests, applies time and cycle bounds, and stops after any failure:

```sh
python3 benchmarks/validation/run.py \
  --runner benchmarks/validation/host/target/debug/zutti-validation-runner \
  --out /tmp/zutti-tree-measurements \
  --case trees-orchard-1-r1 --case trees-orchard-1-r16 \
  --case trees-orchard-4-r16 --case trees-ironwood-1-r1 \
  --case trees-ironwood-1-r16 --case trees-ironwood-7-r16

python3 benchmarks/validation/run.py \
  --runner benchmarks/validation/host/target/debug/zutti-validation-runner \
  --out /tmp/zutti-rejection-checks \
  --case orchard-original-changed-binding \
  --case orchard-fixed-changed-binding --case ironwood-changed-binding
```

Tree cases repeatedly append the entire block's public commitments to a synthetic frontier, starting empty, and compare every resulting root against native code. The action count in a case name is the total for the block, which can exceed that of the selected transaction. These measurements do not authenticate historical frontiers or replace full state-transition validation.

A timeout provides no cycle count. A cycle-limit failure records only a lower bound. Neither is accepted by the estimator as a completed authorization measurement.

`context-orchard-original-2-r1`, `context-orchard-fixed-1-r1` and `context-ironwood-1-r1` measure transaction-context checks, including the shielded coinbase recovery rules. Each also has an `r16` case. Context inputs use a separate domain tag, so an authorization guest cannot be silently substituted for a context guest.

`sapling-1-r1`, `context-sapling-1-r1` and `trees-sapling-1-r1` provide the same separation for a real one-output Sapling coinbase. Each also has an `r16` case. `testnet-coinbase-counts.json` records exact identified shielded coinbase counts in single-transaction blocks. These are lower counts for coinbase recovery work; additional coinbases occur in blocks containing other transactions.

The native `key-cost` binary separates deterministic public-parameter construction from complete verifier-key construction:

```sh
benchmarks/validation/host/target/debug/key-cost --out /tmp/zutti-key-timings
```

It serializes and round-trips the 128 KiB public Vesta parameters, then constructs all three circuit keys. It creates no wallet keys or proving keys. Its wall times help locate setup work; they are not VM cycles or proving latency.

## Sequential guest compatibility

The source guest's transitive dependencies enable Halo2 multicore features. The upstream threaded path attempts to initialize Rayon, which the guest cannot run. The vendored patches select the libraries' existing sequential paths only for `target_os = "zkvm"`. Native configurations and verification equations are unchanged. Each patch records its exact upstream archive and changed-file digests.

Check every vendored file against the cached registry archives:

```sh
python3 benchmarks/validation/check_vendor.py --cache /path/to/registry/cache
```

Changing a guest dependency changes its program identity. These diagnostic results do not update the admitted source program, registry or proved prefix.

## Historical model

`testnet-distribution.json` and `testnet-tree-counts.json` refine the complete inventory pinned to height 4,529,880. Bundle distributions reconcile against the inventory totals. Fixed groups of 128 blocks estimate shared setup frequency; they are not a demonstrated schedule. Witness-size and cycle bounds can require additional splits and setup work.

The modern-authorization model fits shared setup, per-bundle work and per-action work separately for each circuit. It rejects missing circuit types, incompatible programs, incomplete inventories and mismatched history targets:

```sh
python3 benchmarks/validation/estimate.py \
  --measurements /tmp/zutti-original/execution.json \
    /tmp/zutti-fixed/execution.json /tmp/zutti-ironwood/execution.json \
  --out /tmp/zutti-modern-estimate.json

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s benchmarks/validation -p 'test_*.py'
```

This component estimate is not a complete bootstrap budget. Header and ledger processing, other transaction formats, historical state witnesses, paging, recursive proving, proof compression and Ethereum wrapping require separate accounting. A complete execution count requires executing the complete history.

The [recorded model](MODEL.md) combines the completed circuit measurements with explicit proxies for other source work. Reproduce its two planning scenarios with `python3 benchmarks/validation/plan.py --out /tmp/zutti-source-model.json`. They are not guaranteed bounds or a complete proving quote.
