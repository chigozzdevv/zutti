# Source verification

Zutti's selected Testnet model is genesis-bound SPV with complete payment-window coverage. Full-history transaction-validity proving is paused. The active host is in [crates/prover/spv](../crates/prover/spv/README.md).

## Trust boundary

The header program verifies linkage, Equihash, header hash against target, exact expected difficulty, applicable timestamp rules and cumulative work. It starts from the fixed Testnet genesis. Its bootstrap does not accept an operator-selected recent root.

SPV relies on miners enforcing transaction validity. It does not independently prove every shielded proof, signature or ledger transition. Transaction-ID inclusion alone does not prove transaction validity: modern transaction identifiers exclude authorizing data. [ZIP 244](https://zips.z.cash/zip-0244)

The registry additionally enforces its pinned work, observation, confirmation, freshness, supported-height and stable-ancestry rules. A proof cannot establish that an unseen higher-work fork does not exist. Source finality remains probabilistic, and data availability is necessary to observe competing histories and construct proofs.

No committee, custodian or administrator may invent a source checkpoint or select a refund. Unsupported rules, stale progress or conflicting accepted ancestry stop source-dependent execution.

## Complete payment coverage

Each reservation binds one prepared Ironwood payment, its exact effect-committing transaction ID, agreed recipient and amount, source anchor and expiry. The binding journal hides these details behind its commitments; deliberate Testnet witness publication remains a separate privacy boundary.

The window program parses actual complete blocks, derives transaction identifiers, reconstructs transaction commitments and links them to the authenticated ancestry. A list of arbitrary hashes, collapsed subtree or omitted transaction cannot establish absence. Recursive coverage processes consecutive bounded chunks and retains any earlier matching payment.

For an established lower boundary at height 100 and expiry 140:

- Inclusion at 139 or exactly 140 selects payout, even if its destination claim is delayed.
- Refund needs complete absence over 101 through 140 and the required authenticated descendants.
- Coverage ending at 139, a missing claim or an unavailable server is insufficient.

These numbers illustrate the predicate, not deployment confirmation or expiry settings.

The lower search boundary remains a security gate. The [binding construction](payment-window-binding.md) commits the authenticated anchor hash inside the encrypted payment memo. The proposed no-feasible-commitment-cycle argument has not received independent cryptographic review. Collision resistance, mutation tests and a proof of the implemented relation do not by themselves establish that composed argument. Follow the [review packet](refund-window-review.md) before enabling funded refunds.

## Proof and contract composition

The source program authenticates headers and complete window observations. The binding program fixes the payment. The pool program consumes the genuine binding/anchor receipts for reservation and the genuine reservation/window receipts for settlement. Source admission and the private-USDC pool use separate pinned identities and journals.

All proof dependencies must be discharged. An execution result, a serialized source claim or an unresolved assumption is not a receipt. Contract-consumed evidence needs a genuine Ethereum-compatible proof under the exact deployed program/profile. An older genuine receipt cannot certify a rebuilt program.

Public source proving is shared across swaps. The Testnet demo permits public swap witnesses through Boundless/IPFS and makes no claim of confidentiality from provers or other readers of those objects. Signing keys remain local. Confidential prover transport is deferred.

## Recovery and halt behavior

Each party retains the data needed to reconstruct its fixed terminal outcome. Recovery must work without the original website or maker service. A replacement submitter cannot change the payout/refund recipient or infer absence from a timeout.

Unavailable source data or proving capacity can delay settlement. A permanent source conflict halt can leave conditional funds locked; unconditional recovery from that halt is not implemented. Ordinary private notes retain their supported transfer/withdrawal path.

## Acceptance evidence

The [retained evidence](../crates/prover/spv/evidence.json) includes a genuine current-source genesis receipt, historical execution samples and bounded failed continuation attempts. It does not establish current-chain bootstrap, a funded swap or practical ongoing throughput.

Completion requires:

1. Genuine current-image header, admission, binding, coverage and pool proofs, with exact program/journal verification and tampering rejection.
2. Genesis-to-demo-height authentication, fresh subsequent updates and competing-history tests.
3. Real paid, unpaid, expiry-boundary and deliberately delayed-claim cases with complete source coverage.
4. A resolved lower-bound review, matching Sepolia deployment and verified finalized reservation evidence.
5. Live wallet execution, independent recovery and measured complete proof generation, wrapping, memory, cost and ongoing throughput.

Testnet demonstrates behavior under its own consensus and available work. It does not establish production economic attack resistance. [The delivery plan](testnet-delivery.md) defines the implementation sequence.
