# Prepared payment and bounded refund binding

This is an isolated prototype for native shielded ZEC on Testnet. It introduces no committee, custodian or operator-selected history root. It does not authorize funded settlement. The lower-bound construction is conditional on the cryptographic argument below; independent review is still required.

## Checked statement

`PaymentBindingWitness` contains the existing redacted preparation witness and an exact SPV policy. Its evaluator runs the existing Ironwood preparation checks directly. It does not accept a server's payment description or an unverified preparation claim.

The payment must use transaction version 6, the supported consensus branch throughout its inclusion window, Ironwood only, and the required transfer flags. Its nonzero expiry must equal the agreed expiry. The designated positive-value output must match the exact recipient and amount. The evaluator reconstructs its note commitment, ephemeral key and complete encrypted note/memo and compares them with the transaction effects. Source fees remain bounded by the existing preparation predicate.

The encrypted memo contains the deployment domain, the order commitment and the exact source block hash `H0`. The order commits the recipient, amount, both fixed USDC outputs, source policy, anchor height `h0`, expiry `E` and other terms. The named payment profile is `ironwood-v6-anchor-dependency-v1`; a legacy or unrelated profile fails closed.

The statement connects these commitments:

```text
O  = existing private order commitment
 t = exact effect-committing transaction ID
R  = existing reservation commitment to (O, t)
W  = (domain, O, h0, H0, E, fixed payout, fixed refund)
Rw = SPV reservation commitment to (policy, W, t)
```

The same private transaction ID and reservation randomness produce `R` and `Rw`, using different commitment domains. `ZUTTIBND` version 1 publishes only the deployment domain, order commitment, `R`, source policy identity, digest of `W`, `Rw` and payment-profile identity. It omits the transaction ID, recipient, amount, memo and branch selector.

`ZUTTIBWI` version 1 encodes the witness. Redaction removes spending keys, viewing material, input-note openings, signatures and transaction proofs that preparation does not need. Output openings remain sensitive. Authorization may subsequently change without changing the prepared effects or either reservation commitment.

## Lower-bound argument

This construction does not treat a memo or a block height as a consensus creation timestamp. Its proposed lower bound is a hash dependency on an authenticated source block.

The pinned transaction-ID implementation covers the designated output ciphertext, including the encrypted memo. Version 6 moves shielded note-tree anchors into authorizing data; that does not remove the memo ciphertext from the effect commitment. A mutable note-tree anchor is not the block-hash dependency used here. [ZIP 244](https://zips.z.cash/zip-0244), [ZIP 229](https://zips.z.cash/zip-0229)

Assume that `H0` is authenticated at height `h0` on the accepted ancestry. The binding preparation relation fixes a correctly recoverable output whose encrypted memo contains that exact `H0`. If the same qualifying transaction appeared at or before `h0`, its transaction ID would feed a transaction Merkle root in `H0`'s own ancestry. The dependency would run from `H0` through the checked encrypted output and transaction ID, then back through the ancestor commitments to `H0`.

The explicit cryptographic assumption is that an adversary cannot feasibly construct this commitment cycle or reopen the same committed output as a different anchor-bearing payment. This relies on the composed hash/commitment and output-encryption construction. Collision resistance alone is not a formal proof of this claim. The repository's mutation tests check the implemented relation; they do not establish the computational hardness assumption. Independent cryptographic review remains an admission gate.

Under that assumption, an authenticated matching transaction cannot occur at or before `h0` on the accepted ancestry. Its committed nonzero expiry excludes valid first inclusion after `E`, relying on miners enforcing Zcash transaction validity under the selected SPV model. Complete absence over `h0 + 1` through inclusive `E` therefore covers every possible payment block. Accepted stable source progress and the required descendants must also be established. Source absence alone is insufficient if the preparation/binding receipt or accepted ancestry is missing.

## Composition required for settlement

A funded transition must verify both genuine receipts under the pinned binding and source programs. It must match the source policy and `Rw`, the exact window terms, and the selected fixed output. `PaymentBindingClaim::require_window` checks this field relation; it is not a receipt verifier.

The source window receipt authenticates the anchor and consecutive history, selects inclusion or complete absence, and binds the exact ending checkpoint. The destination must admit that checkpoint through the source registry. Funding must also check source freshness and remaining inclusion time.

The private-pool proof must separately establish ownership, backing, conservation, membership, fixed encrypted output data and the shared terminal nullifier. The payer's authorization gate and Ethereum reservation-finality checks still apply. A delayed paid claim must remain available; a timer or missing claim must never authorize refund.

## Implementation and evidence

The evaluator is in [`crates/wallet/src/binding.rs`](../crates/wallet/src/binding.rs). The separate [binding guest](../crates/prover/methods/binding/src/main.rs) is built independently of the admitted pool, preparation and source images. Existing production proof-kind admission is unchanged.

The [public demo](../benchmarks/spv/examples/payment-binding.rs) generates Ironwood PCZTs with test-only keys and synthetic terms. Its source-program identifier is a placeholder, not a source checkpoint. Nothing is broadcast and no real user witness is uploaded. It checks correct binding and guest rejection of changed recipient, amount, expiry, anchor and height. Native tests additionally cover source-policy/profile substitution, ciphertext mutation, authorization stability, valid re-encryption with a different anchor, and malformed encodings.

The [2026-10-24 results](../benchmarks/spv/results/2026-10-24-binding/manifest.json) record the isolated image, program hash, source hashes and all six execution outcomes. On the Apple M1 Pro, the valid case used **54,801,665 user cycles** and **4.861 seconds of execution**. The six-case supervised run took 27.643 seconds and sampled a 90.844 MiB peak process-group RSS. Sampling can miss brief memory spikes. These figures are not proof-generation time or a per-swap latency estimate.

The combined Rust suite passed 38 tests, including eight binding tests; clippy passed with warnings denied. The unchanged contract suite passed 28 tests with one existing skipped test; SPV contract proof verification remains mocked. The build and execution used cached dependencies and generated public inputs, with no new chain downloads or paid jobs. [Reproduction commands](../benchmarks/spv/README.md#prepared-payment-binding)

A successful guest execution is not a generated proof. Genuine binding receipts, current-chain source proving, Ethereum wrapping and funded private settlement remain separate milestones. A hosted private prover learns the witness unless a validated confidentiality mechanism protects it.
