# Sapling signature execution profiling

This guest compares individual Sapling binding-signature verification through the current RedJubjub 0.9 library and the source verifier's canonical bridge to RedJubjub 0.8. The latter uses the same pinned RISC Zero BLS12-381 field backend and vendored Jubjub compatibility patch as the accelerated tree implementation. The bridge supplies encoded public keys and signatures, with the original message unchanged. No randomized batching is used.

The current and older RedJubjub verification implementations use the same challenge personalization, canonical point/scalar decoding and cofactored verification equation. The native differential suite additionally checks genuine historical spend and binding signatures and 1,512 encoding/scalar/cofactor combinations, including negative-zero point encodings and scalars around the exact modulus. The source validator retains its separate small-order spend-key rejection. Binding-key construction and transaction parsing still use the current Sapling library.

`bindings.json` contains four genuine Mainnet signatures from the pinned `zebra-test` 4.0.0 block vectors. Each entry names its block and transaction. The host verifies every selected fixture with the current library before guest execution. The count is bounded to 1–64 and cannot exceed the fixture size; duplicate selected records are rejected.

```sh
cargo run --locked -p zutti-prover --example sapling-signature-profile -- \
  --fixture benchmarks/sapling-signatures/bindings.json \
  --out /tmp/zutti-sapling-signatures-reference --count 4 --check-invalid

cargo run --locked -p zutti-prover --example sapling-signature-profile -- \
  --fixture benchmarks/sapling-signatures/bindings.json \
  --out /tmp/zutti-sapling-signatures-accelerated --count 4 --accelerated --check-invalid
```

The input contains a one-byte count followed by fixed-width records: a 32-byte public key, 32-byte digest and 64-byte signature. The journal must equal SHA-256 of the entire input. `--check-invalid` requires guest rejection of a changed message, noncanonical key, noncanonical R encoding and noncanonical signature scalar. Other execution failures do not count as rejection. Both executions retain exact programs, inputs, journals, artifact hashes, user cycles and padded segment cycles. Execution is capped at 2^31 cycles.

```sh
cargo test --locked -p zutti-source --features accelerated-sapling-signatures \
  --lib authorization::signatures
cargo test --locked -p zutti-source \
  --features storage,windowed-inputs,accelerated-trees,accelerated-scripts,accelerated-sapling-signatures \
  --test authorization --test signatures --test state --test transition
```

These are public component diagnostics. They do not generate receipts, admit source history or authorize settlement. Component cycle savings are not a full-bootstrap budget, proving-latency result or market quote. A new source image needs its own proved ancestry.

The [recorded comparison](../source-sapling-signatures-2026-10-19.json) used identical input and output bytes: 66,503,115 cycles for four reference verifications and 12,049,510 for the accelerated adapter, an 81.88% reduction. Both guest versions rejected all four invalid-input cases. Full-block measurements are tracked separately in that record.

The real NU5 activation block at Testnet height 1,842,420 also completed with an identical expected journal in 94,934,070 cycles. Its version-5 coinbase has no Orchard actions or other shielded authorization proofs. This checks the upgrade transition; Orchard-heavy throughput remains unmeasured. The prior native state still has no genesis-anchored proof.

The complete Testnet block at height 282,757 matched the same expected journal in 82,755,985,931 cycles across 84,142 segments. Its preceding transparent-signature-optimized image used 102,198,405,440 cycles: a 19.02% reduction for this full block. The earlier windowed-Groth16-only image used 108,667,191,925 cycles. These stages reduce the observed workload by 23.84%, and it still exceeds the normal job guard. The comparison used a local 2^37 diagnostic ceiling and generated no block receipt.
