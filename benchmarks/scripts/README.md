# Transparent signature execution profiling

This diagnostic compares Zcash's existing transparent signature verifier with the source guest's `accelerated-scripts` adapter. Both use `zcash_script` 0.6.0 signature decoding and the same precomputed transaction digest. The adapter retains `secp256k1` 0.33.1 public-key parsing and high-S normalization, then verifies the canonical key and signature with `k256` 0.13.4. It does not hash the digest again. The transaction interpreter, script flags, sighash construction and lock-time checks remain unchanged.

The guest pins the official [RISC Zero k256 fork](https://github.com/risc0/RustCrypto-elliptic-curves/tree/7d4a8d6477e258ce4169ee4669cf92aee0582c39) and [crypto-bigint fork](https://github.com/risc0/RustCrypto-crypto-bigint/tree/3ab63a6f1048833f7047d5a50532e4a4cc789384). Zcash permits high-S signatures, while k256's verifier rejects them; normalization before the bridge is required for equivalent acceptance. Parsing with the existing library also preserves compressed, uncompressed and hybrid public-key acceptance.

Run from the repository root with the RISC Zero toolchain and `r0vm` installed:

```sh
cargo run --locked -p zutti-prover --example script-profile -- \
  --out /tmp/zutti-script-reference-16 --count 16 --check-invalid

cargo run --locked -p zutti-prover --example script-profile -- \
  --out /tmp/zutti-script-accelerated-16 --count 16 --accelerated --check-invalid
```

Inputs are deterministic public test signatures, with all three public-key encodings, low- and high-S signatures, and digest boundary cases. The example keys are reproducible and must never hold funds. No wallet or swap witness is loaded. The runner checks every valid fixture with the native reference, saves the program, input and journal, and requires the guest's entire journal to equal the expected input digest. `--check-invalid` also requires guest rejection of a changed digest, malformed key, malformed DER and zero signature scalars. An infrastructure error does not count as rejection.

The count is bounded to 1–64 and execution to 2^31 cycles. Output directories must not already exist. `execution.json` records user cycles, padded segment cycles, image and artifact hashes, and wall time. `rejections.json` records the checked failures. These runs do not generate receipts, admit source history or authorize settlement; execution time is not proving latency.

Native compatibility tests and source regressions:

```sh
cargo test --locked -p zutti-source --features storage,accelerated-scripts \
  --lib transaction::
cargo test --locked -p zutti-source \
  --features storage,windowed-inputs,accelerated-trees,accelerated-scripts \
  --test signatures --test authorization --test state --test storage \
  --test replay_store --test transition
```

The [measurements](../source-scripts-2026-10-19.json) separate component work from complete block execution. Full source comparisons retain the old statement's expected journal and identify the candidate's different image. Matching that journal does not produce a receipt for the old image. The new policy requires its own proved ancestry before source admission.

A separate local run generated a genuine succinct receipt for the one-signature accelerated program in 428.79 seconds. Its [public fixture descriptor](../../tests/fixtures/script-accelerated-receipt.json) binds the receipt, exact input and image hashes. The regression test verifies it with development receipts disabled and rejects a different image, changed journal and altered seal. This proves the diagnostic signature program's execution; it does not validate a block or establish source-history ancestry. No paid request or Ethereum wrapper was used.

```sh
cargo test --locked -p zutti-prover --test accelerators
```
