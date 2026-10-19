# Sapling tree execution profiling

This diagnostic guest measures Sapling commitment-tree hashing. Its generated public inputs contain a hash count and two fixed 32-byte children. Each hash becomes the next left child; the depth cycles through all 32 tree levels. The host checks the final value against `sapling-crypto` 0.9.0. The runner records the exact program, input, journal, cycles and padded segment work. It does not generate receipts, validate blocks or authorize settlement.

The `windowed-trees` implementation uses the same Pedersen hash and published subgroup generators. Its 516 bits are the six depth bits followed by 255 bits from each child. Each signed three-bit chunk contributes `±(1 + a + 2b) × 16^i` times its generator. The first two generators each handle 63 chunks; the third handles 46. Adjacent chunks from the same generator share a lookup of `16 × high ± low`, with the high chunk's sign applied outside the lookup. Each pair needs 32 points; two unpaired chunks need four each. The guest derives 2,728 affine lookup points once, and each hash uses 87 point additions. No lookup points or intermediate arithmetic are supplied by a witness. The curve remains Jubjub 0.11.1 over BLS12-381 0.9.0.

Source validation separately caches unchanged note-tree roots. Those caches are not serialized or committed. Decoding clears them, appending invalidates the affected pools, and failed appends preserve the original state. Both optimizations retain the existing frontier encoding and resulting commitments.

Run from the repository root with the RISC Zero Rust toolchain and `r0vm` installed:

```sh
cargo run --locked -p zutti-prover --example tree-profile -- \
  --out /tmp/zutti-tree-reference-32 --count 32

cargo run --locked -p zutti-prover --example tree-profile -- \
  --out /tmp/zutti-tree-windowed-32 --count 32 --windowed
```

Use a larger count to measure repeated work after initialization. Execution is capped at 2^31 cycles; output directories must not already exist. Wall time is neither proving latency nor a fee quote.

```sh
cargo test --locked -p zutti-source --features storage,windowed-trees --lib trees::
cargo test --locked -p zutti-source --features storage,windowed-inputs,windowed-trees \
  --test authorization --test state --test storage --test replay_store --test transition
```

The differential suite checks 320 child pairs across every depth against the upstream hash and 384 sign patterns at paired windows and generator boundaries, plus child-bit boundaries, pinned empty nodes, Zebra tree results, cache invalidation, serialization and restart behavior. Full source comparisons use `source-prover compare` with exact retained public inputs. A diagnostic candidate's different image cannot provide a receipt for an older program-bound statement. A new source policy still needs its own proved ancestry; matching output bytes does not admit a checkpoint or migrate a policy.

## Measurements

The [retained measurements](../source-trees-2026-10-19.json) include the superseded single-chunk prototype and final paired implementation. Exact inputs and outputs matched for each comparison.

| Workload | Original cycles | Final paired cycles |
| --- | ---: | ---: |
| 32 chained tree hashes | 863,362,712 | 150,184,447 |
| 256 chained tree hashes | 1,350,306,415 | 450,979,491 |
| Testnet Blossom activation block 584,000 | 991,639,926 | 209,343,440 |

Subtracting the 32-hash run from the 256-hash run gives 38.23% fewer cycles for the additional 224 hashes. This observed difference separates repeated work from most initialization overhead; it is not a full-bootstrap projection. The real Blossom block contains one coinbase transaction and no shielded authorization proofs, so its 78.89% reduction measures tree and initialization improvements.

The 108.667-billion-cycle Sapling stress-block result in the [authorization measurements](../source-authorization-2026-10-19.json) belongs to the earlier source image. That complete busy block has not been remeasured with paired trees. No receipts, paid requests, policy migration or source admission are performed by these comparisons.
