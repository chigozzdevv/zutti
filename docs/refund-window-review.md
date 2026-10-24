# Bounded refund review packet

Status: review prepared; independent cryptographic review is not complete. This packet identifies the proposition to establish. It does not approve funded refunds.

Scope: the version-6 Ironwood payment profile `ironwood-v6-anchor-dependency-v1`, the pinned preparation implementation, the SPV window relation and the composed pool settlement relation. The source model relies on miners enforcing transaction validity and on relevant competing histories reaching source admission.

## The proposition

Let `C` be the accepted source ancestry, `H0` its authenticated block hash at height `h0`, and `E` the agreed nonzero expiry. Let `t` be the exact transaction ID hidden in the binding and source-reservation commitments.

The proposed conclusion is:

```text
valid binding to (C[h0] = H0, t, E)
and complete authenticated absence of t in C[h0 + 1 .. E]
imply that the qualifying payment did not occur in C
and cannot first occur later in C while remaining valid.
```

This needs two different boundaries. Expiry supplies the consensus upper boundary: first inclusion after `E` is invalid, while inclusion at `E` is allowed. [ZIP 203](https://zips.z.cash/zip-0203)

The lower boundary is the part requiring cryptographic analysis. The prepared output's correctly constructed encrypted memo includes `H0`, the deployment domain and the private order commitment. The transaction identifier commits to the checked output ciphertext. If that same transaction were present at or before `h0`, the source ancestry would also commit to it, creating a dependency from `H0` through the payment and back to `H0`.

The candidate assumes that an adversary cannot feasibly construct that cycle or find an alternate valid output opening for the same effect commitment. Collision resistance alone is not a proof of that composed claim. A random-oracle-style argument, if proposed, must name the exact modeled functions, attacker control and security bound rather than treating the desired conclusion as an assumption about honest wallet behavior.

## Checked implementation path

| Relation | Implementation | What must be established |
| --- | --- | --- |
| Fixed order | `crates/protocol/src/order.rs` | Recipient, amount, expiry, anchor, deployment and both USDC outputs are committed consistently |
| Recoverable payment output | `crates/wallet/src/prepare.rs::inspect_bound_output` | The note commitment, ephemeral key and entire encrypted plaintext/memo match the chosen note opening and order |
| Effect identifier | Pinned `zcash_primitives` transaction digester | The exact version-6 Ironwood output ciphertext and expiry are effect-committing data; later authorizations cannot change their meaning |
| Both reservations | `crates/wallet/src/binding.rs` | The same hidden `t`, order and randomness bind the ordinary reservation and salted source reservation |
| Source anchor | `crates/source/src/spv/chain.rs`, admission guest and registry | `H0` belongs to the genesis-bound accepted ancestry under the exact source policy |
| Complete interval | `crates/source/src/spv/{window,coverage}.rs` | Every eligible block is covered with all transaction identifiers; paid prefixes cannot be discarded during composition |
| Fixed entitlement | `crates/wallet/src/spv_pool.rs` | A genuine reservation certificate and matching genuine window select exactly one fixed output and consume the same terminal nullifier |

Transaction-ID field behavior must be checked against the pinned implementation and the version-6 amendments, not inferred solely from the older Orchard layout. [ZIP 244](https://zips.z.cash/zip-0244), [ZIP 229](https://zips.z.cash/zip-0229)

## Why lock time is not the current solution

The pinned `zebra-chain` 14.0.0 implementation returns no effective lock-time restriction unless a transparent input sequence enables one. Our payment profile rejects transparent inputs and outputs. Merely putting a future height in `nLockTime` therefore does not establish the desired lower boundary.

This was checked in `transaction.rs::lock_time` at package source revision `6d1e414d6f55e4180d0e47baaa934bf97d5b4fec`. The corresponding consensus check accepts the no-effective-lock-time case. [Pinned implementation](https://github.com/ZcashFoundation/zebra/blob/6d1e414d6f55e4180d0e47baaa934bf97d5b4fec/zebra-chain/src/transaction.rs#L128), [consensus check](https://github.com/ZcashFoundation/zebra/blob/6d1e414d6f55e4180d0e47baaa934bf97d5b4fec/zebra-consensus/src/transaction/check.rs)

A shielded note-tree anchor is also not interchangeable with `H0`: version 6 permits shielded anchor/proof updates as authorizing data without changing the transaction identifier. The current construction uses the encrypted memo dependency instead. [PCZT anchor rules](https://zips.z.cash/zip-0374#anchors-and-pre-authorization)

## Adversary and review questions

The adversary may control either RFQ party, all locally chosen note/encryption randomness, prepared payment data, proving inputs, relays, data providers and proof submission timing. Do not assume an adversarial sender uses our wallet's normal randomness or waits until the honest preparation step. Chain/proof/commitment security assumptions must be stated separately from that attacker control.

| Attack or boundary | Required review outcome |
| --- | --- |
| Previously mined payment reopened under a later anchor | Establish infeasibility for the complete note/encryption/effect-commitment relation, or produce a counterexample |
| Sender-controlled recipient, ephemeral key, note randomness and memo | Analyze alternate openings, key equivocation and chosen-ciphertext construction; honest decryption examples are insufficient |
| Transaction in the anchor block itself | Include the same-block cycle, not only earlier ancestor blocks |
| Changed shielded proof, authorization or note-tree anchor | Show which changes preserve `t` and why they cannot replace the committed payment/output/expiry |
| Wrong transaction version or network upgrade | Bind the supported branch for the entire eligible interval and reject a crossing interval or unsupported version |
| Same `t` substituted across orders or deployments | Check ordinary/source reservation linkage, domain separation, exact fixed outputs and funding-certificate composition |
| Missing, duplicate, reordered or truncated block data | Require complete transaction-root reconstruction and consecutive interval coverage; a lookup result is insufficient |
| Earlier paid chunk followed by an absent chunk | Preserve the exact earlier coverage receipt and paid state through recursion |
| Payment mined exactly at `E` or proof presented later | Keep the paid entitlement; destination time cannot change its owner |
| Alternate fork or stale source prefix | Require the accepted ancestry and declared observation policy; proving one valid chain does not prove no unseen heavier chain exists |
| Loss of a prover, relay or original maker | Preserve entitlement using retained records, the funding certificate and independent submission; availability failure cannot manufacture absence |
| Permanent source halt | State that conditional funds can remain locked; an automatic refund is not a justified recovery mechanism |

## Evidence and its limits

Existing mutation tests reject changed recipient, amount, expiry, anchor/hash, profile and output ciphertext. They check authorization stability and valid re-encryption under a changed anchor, and they reject reuse of the old binding. Source tests cover complete interval processing, boundary inclusion and forged continuation data. The [recorded binding results](../benchmarks/spv/results/2026-10-24-binding/manifest.json) and [coverage results](../benchmarks/spv/results/2026-10-24-coverage/manifest.json) label their actual evidence classes.

These tests do not demonstrate that a computationally bounded adversary cannot construct a commitment cycle. A genuine proof of the implemented predicate would not establish that theorem either. Existing conditional executions use unresolved assumptions; they are not genuine composed payment/refund proofs.

## Review completion

Retain the exact reviewed source revision, program identities, modeled assumptions, attack analysis, findings and their resolution. Independent review must address the lower-bound proposition and the composed funding/window/settlement relation, rather than only inspecting the memo format. Any blocking finding prevents admission of funded refunds until the construction is repaired and its affected profiles and proofs are regenerated.

This is a focused review of refund soundness, separate from a complete production security audit.

The review may conclude the proposed relation is sound under named assumptions, needs modification, or does not establish the claimed boundary. No outcome is presumed here. The [delivery plan](testnet-delivery.md) allows independent implementation work to continue while this gate is resolved.
