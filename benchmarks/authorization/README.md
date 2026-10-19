# Authorization execution profiling

This standalone guest measures individual Groth16 verification equations using the production implementation and embedded Zcash verifying keys. It does not validate a transaction or block, prove source history, or authorize settlement. Its image is not admitted as a source verifier.

`sapling-outputs.json` contains eight distinct Sapling output proofs from Testnet block 282,757. `sapling-outputs-64.json` contains 64 distinct outputs from that block. Each entry records its transaction ID, output index, proof and five canonical little-endian public inputs. Both fixtures record the retained source witness's SHA-256 digest. Counts above a fixture's size repeat it cyclically; they measure repeated verification work, not additional distinct transactions.

`sapling-spend.json` and `sprout-groth16.json` contain genuine Mainnet proofs from `zebra-test` 4.0.0's block vectors, with seven and nine public inputs respectively. Their block heights and transaction IDs are recorded in each fixture.

The reference uses `bls12_381` 0.9 and `groth16` 0.2. The accelerated backend uses a canonical input bridge to `bls12_381` 0.8 and Bellman 0.14. The guest pins [RISC Zero's acceleration fork](https://github.com/risc0/zkcrypto-bls12_381/tree/a3192e1bc58921d42d03d99957abf3be55f9d435). See the [official precompile documentation](https://dev.risczero.com/api/zkvm/precompiles) for its supported versions.

The windowed backend prepares 16 multiples of each public-input basis point once. Four-bit interleaving shares doubling steps across the inputs. Every proof retains its own complete Groth16 pairing equation, canonical decoding and subgroup checks. No randomized batch coefficients or prover-supplied arithmetic results are accepted.

Run from the repository root with the RISC Zero Rust toolchain and `r0vm` installed:

```sh
cargo run --locked -p zutti-prover --example authorization-profile -- \
  --fixture benchmarks/authorization/sapling-outputs.json \
  --out /tmp/zutti-reference-4 --count 4

cargo run --locked -p zutti-prover --example authorization-profile -- \
  --fixture benchmarks/authorization/sapling-outputs.json \
  --out /tmp/zutti-accelerated-4 --count 4 --accelerated

cargo run --locked -p zutti-prover --example authorization-profile -- \
  --fixture benchmarks/authorization/sapling-outputs.json \
  --out /tmp/zutti-windowed-4 --count 4 --accelerated --windowed

cargo run --locked -p zutti-prover --example authorization-profile -- \
  --fixture benchmarks/authorization/sapling-outputs-64.json \
  --out /tmp/zutti-windowed-64 --count 64 --accelerated --windowed
```

Add `--profile` to retain a cycle profile. Output directories must not already exist. The runner builds a locked diagnostic guest, retains its exact program and input, and checks its journal against SHA-256 of that input. It caps execution at 2^31 session cycles and never invokes a prover or purchases a proof. User cycles and padded segment cycles are recorded separately. Execution wall time is not proving latency or a fee estimate.

Native differential checks compare genuine proofs and changed statements against the 0.9 reference verifier. Window arithmetic is also checked against independent scalar multiplication for all three embedded Groth16 consensus keys:

```sh
cargo test --locked -p zutti-source --features windowed-inputs --lib authorization::groth
cargo test --locked -p zutti-source --features windowed-inputs --test authorization
```

## Complete source execution

`source-prover compare` runs an exact retained source input against a candidate program and requires its complete output journal to match. Its default cycle limit is 2^31. A larger explicit `--cycle-limit` is permitted for local diagnosis, up to 2^38; it does not change proof generation, exported jobs or Boundless spending limits.

```sh
source-prover compare --job /path/to/retained-public-job \
  --candidate /path/to/source/program.elf --cycle-limit 137438953472
```

This command executes without proving or admitting source state. A candidate with a different image cannot supply a receipt for the retained job's older program-bound statement. Starting-state continuity to genesis still requires a proved prefix.

## Proving large validations

The 2^31 limit is Zutti's current job guard, not a Zcash block-validity rule. [RISC Zero already constructs segment proofs and recursively joins an entire session](https://dev.risczero.com/proof-system/). [Bento schedules segment generation, prove/lift, join and resolution tasks](https://docs.boundless.network/provers/bento). These mechanisms preserve the program's execution continuity without requiring a new Zcash-specific sub-block protocol.

Provider capacity and pricing remain separate checks. Boundless brokers can impose a [maximum mcycle limit](https://docs.boundless.network/provers/broker) and decline larger requests. A bigger request also retains the underlying computation. The completed 108.667-billion-cycle local run establishes execution work; it does not establish a market quote, fulfillment time or an affordable bootstrap. Paid limits remain unchanged while those measurements are unresolved.
