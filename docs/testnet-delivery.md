# Zutti Testnet delivery

Agreed target: a working cross-chain RFQ swap between native shielded ZEC on Zcash Testnet and private USDC notes backed by Sepolia USDC. Settlement must use genuine cryptographic evidence. Full historical transaction-validity proving remains paused; the selected source model is SPV with the mining-validity and chain-observation assumptions in [source verification](source-verification.md).

This file is the delivery sequence. The [settlement construction](spv-settlement.md) describes the current predicates. Passing component tests or executing a guest with unresolved assumptions does not complete any proof or live-swap milestone below.

## Fixed decisions

- No custodian, checkpoint committee, operator-selected recent source root or discretionary refund authority.
- RFQ fixes the commercial terms. A price feed cannot change an accepted deal or decide settlement.
- Prepare and bind the exact shielded payment before reserving USDC. Sign only after independently verifying the finalized reservation and retaining its funding certificate.
- A payment within the valid window selects the agreed payout. Complete authenticated absence through inclusive expiry selects the agreed refund. A missing claim or elapsed destination timer selects neither.
- Both outcomes consume one escrow nullifier and create the exact proof-bound output. A delayed paid claim remains eligible.
- Public source history can be proved separately and reused. Private swap witnesses stay local; the currently authorized public proving transport accepts generated public demo inputs only.
- No new paid proving, GPU rental or proving-setup download is authorized by this plan. Compute availability and demonstrated proving throughput remain delivery dependencies.

## Work and completion evidence

| Order | Work | Existing starting point | Required completion evidence |
| --- | --- | --- | --- |
| 1 | Resolve the bounded-refund security gate | `crates/wallet/src/binding.rs`, `prepare.rs`, `crates/source/src/spv/window.rs` | Written analysis and independent review of the actual lower-bound construction, explicit assumptions and resolved blocking findings; [review packet](refund-window-review.md) |
| 2 | Freeze programs and produce genuine component proofs | Isolated binding/source/admission/pool guests; `benchmarks/spv/src/{binding,settlement,ethereum}.rs` | Exact release images, journals, closed proof dependencies, private zero-knowledge receipt formats, verified Ethereum seals and real-verifier rejection of tampering |
| 3 | Bootstrap and maintain authenticated Testnet headers | `crates/source/src/spv/`, generic SPV witness execution/proving | A recoverable genesis-to-demo-height receipt chain, fresh accepted source updates and demonstrated ability to process subsequent blocks |
| 4 | Deploy and admit the Sepolia contracts | `contracts/src/{SpvRegistry,SpvPool}.sol`, native Ethereum verifier | Pinned deployment manifest, actual Groth16 verification, accepted genuine source updates and independently verified finalized reservation storage |
| 5 | Connect wallets and SDK fulfillment | `SpvPaymentJournal`, `spv_authorization.rs`, SDK/RFQ packages | Real funded source notes, native transaction proving/signing/broadcast, private USDC account operations and an operational `intent.accept()` lifecycle |
| 6 | Run the complete swap and recovery cases | Recovery CLI and the composed predicates | Real chain transactions and genuine receipt bundles for paid, unpaid, delayed-claim and independent-recovery cases |
| 7 | Make the flow usable and submit it | RFQ relay, client/maker SDKs | A working judge-facing demo, two-minute video and public source with reproducible setup and accurate security boundaries |

The order expresses dependencies, not a requirement to leave independent work idle. Source-job preparation, deployment tooling and wallet integration can progress while proof capacity and review are being established. Funded execution waits for its security and evidence gates.

## 1. Bounded-refund security gate

The question is whether a qualifying prepared payment can have existed at or before the authenticated anchor while still passing our binding relation. Expiry only supplies the upper boundary. Complete absence over a short interval is insufficient until its lower boundary is justified.

Work through the [review packet](refund-window-review.md), including sender-controlled encryption randomness, alternate output openings, authorizing-data changes, fork substitution and the exact version-6 transaction-ID implementation. Mutation tests and a generated ZK proof check the implemented relation; neither proves the proposed cryptographic hardness argument.

The current anchor-hash dependency is a candidate requiring review. It is not promoted to a consensus-enforced creation timestamp. `nLockTime` is not an automatic substitute: the pinned Zebra implementation disables its lock-time condition when no transparent input enables it. Adding a transparent leg is not an approved change to the payment profile.

If the argument fails or a blocking finding remains, repair the construction and rebind its profile before funding conditional escrows. Do not expand the claimed guarantee by relabeling the assumption as a disclosure.

## 2. Genuine proof chain

Freeze the source policy, supported-height horizon, payment profile, proof parameters and all program identities in one release manifest. A changed image requires matching receipts and a matching deployment profile. Old genuine fixtures cannot certify a new program.

The required proof relations are:

```text
genesis -> header-prefix receipts -> source-admission receipt -> accepted checkpoint

prepared Ironwood payment -> binding receipt
binding receipt + accepted anchor receipt + authorized inventory -> reservation receipt

accepted anchor + complete payment interval -> paid/refund window receipt
reservation receipt + window receipt + escrow membership -> settlement receipt
```

Private USDC deposit, transfer and withdrawal also need genuine proofs under the SPV pool's exact image and journal. The earlier private-pool receipts use a different image/journal and cannot be substituted.

Public header receipts may be composite. Private binding, interval coverage, reservation and settlement dependencies must use genuine succinct or Groth16 receipts. All assumptions must be discharged; development-mode or fabricated receipts fail acceptance. Every contract-consumed proof needs a genuine Groth16 wrapper checked against its exact image and journal.

Zcash transaction validity has a separate proving requirement: the native Ironwood transaction needs its own shielded proof and spend authorization. A Zutti binding proof does not replace that transaction proof. Ethereum finality and storage evidence are verified by the native Ethereum adapter; a service reporting that a reservation is confirmed is insufficient.

Measure complete proof generation, dependency composition and wrapping for the actual programs. Record cycles, elapsed time, memory, proof format and paid fees separately. No complete bootstrap price or swap latency is established by the existing execution measurements.

## 3. Authenticated source operation

Inventory the existing Testnet archive, current node tip and genuinely verified prefix. Reuse existing public data on the source host; do not assume an old inventory or a node database is already a proof.

Implement the operational SPV path around the existing predicates:

1. Export consecutive raw headers, the fixed genesis and the exact source policy.
2. Prepare bounded header batches and persist their expected input/output identities.
3. Prove each batch against its exact genuine predecessor, or start the first batch directly from genesis.
4. Verify and persist the returned receipt before advancing the durable cursor. Restart must reuse and reverify completed work.
5. Produce source-admission receipts and advance the registry under its work, ancestry, observation and freshness rules.
6. Continue proving fresh blocks while swaps are active. Build complete-block window witnesses from that same ancestry; use recursive coverage when an interval exceeds one input's byte limit.

The [recoverable SPV bootstrap runner](spv-bootstrap.md) now exports bounded headers from the existing node, prepares immutable jobs, accepts only genuine exact-journal receipts and reverifies accepted work on restart. Automatic catch-up, source admission/submission, fork monitoring and demonstrated current-chain proving throughput remain unfinished. The full-validity bootstrap tooling does not automatically implement this selected SPV pipeline.

Choose batch sizes, expiry, minimum remaining blocks and freshness limits from measured proof/update behavior and a reviewed source policy. Preserve the current `expiry + 2 * confirmations` supported-height requirement. Do not reduce confirmation or chain-observation protections merely to make a recording finish faster.

## 4. Deployment and admission

Pin actual Sepolia chain identity, official test USDC address/decimals, verifier parameters, source policy, program images, contract addresses and runtime code hashes. Keep deployment signing keys outside the repository. Deploy the real verifier, SPV registry and SPV pool; retain deployment transactions and independently verify the resulting configuration.

Demonstrate genuine source admission before funding swaps. Use finalized Ethereum execution/account/storage evidence to verify the exact escrow, order-use marker and unspent terminal nullifier. Pin and validate the Ethereum consensus bootstrap configuration and checkpoint/finality ages.

Contract acceptance tests must execute genuine proofs against the deployed verifier and reject changed images, journals, outputs and seals. An EVM test that replaces proof verification does not satisfy this milestone.

## 5. Wallet and SDK fulfillment

Generate dedicated Testnet wallets when implementing the live wallet provider. Keep seeds and signing material private. Report the public funding addresses and required test ZEC, Sepolia ETH and test USDC before asking for funding; do not request mainnet funds.

Connect source scanning, spendable-note selection, PCZT preparation, native Ironwood proving, authorization, broadcast and exact-byte retries. Connect private USDC scanning, balances, backed deposits, reservation, receipt recovery and withdrawals. Persist recovery material before irreversible actions.

Before the USDC funder signs an inventory spend, check the locally accepted terms, expected Zcash recipient/amount and the funder's own private refund receiving key against the reservation. Genuine evidence for a different deal does not establish user consent. This funder-side gate is separate from the existing ZEC-payer authorization adapter. Maker balances, active reservations and outstanding swaps must come from actual wallet/chain state rather than placeholder counters.

Connect `intent.accept({ offer_id, on_status })` through the native execution adapter using the agreed SDK shape. It must verify immutable accepted terms and the intended receiving key, retain and verify the funding seal, verify source freshness and finalized Ethereum evidence, then coordinate signing and settlement. Current `accept()` refuses execution, and current maker inventory/active-swap reporting is not a complete settlement service.

Expose genuine stages and final outcomes. A proof submission, broadcast or pool deposit must not be reported as a completed swap. The relay delivers encrypted RFQ messages; it has no authority to decide payment or refund.

## 6. Live acceptance cases

| Case | Required result |
| --- | --- |
| Paid | A real shielded Testnet payment is mined within the bound window; genuine evidence creates the fixed private USDC payout |
| Unpaid | No qualifying payment occurs; complete authenticated coverage through inclusive expiry creates only the funder's fixed refund |
| Delayed claim | The payment is mined in time, but its destination claim is withheld beyond expiry; refund evidence fails and the payout remains available |
| Boundary inclusion | Inclusion exactly at expiry remains paid; a premature absence or omitted interval block fails |
| Independent recovery | Stop the normal maker/relay process; retained encrypted records and genuine evidence permit another submitter to complete the same fixed outcome |
| Reverse trade | Exercise the same invariants with the RFQ parties' ZEC-payer/USDC-funder roles reversed |
| Private balance | The recipient can recover and spend the resulting private note; a public withdrawal delivers its actual backing and prevents replay |

Retain proof/image/journal identities, source checkpoints, destination transaction hashes and encrypted owner recovery data. Publish proof evidence without silently publishing private witnesses or transaction-to-escrow links. Any deliberately disclosed test data must be labeled as such.

## Source halt behavior

Distinguish unavailable/stale data from a proved conflict. Missing data is not absence and cannot authorize a refund. Freshness failure stops new reservations and prevents source advancement until valid evidence is available. A heavier conflicting stable history triggers the registry's existing permanent safety halt and invalidates its accepted-checkpoint lookups.

No timeout or administrator can choose a conditional escrow's owner after a halt. Ordinary private notes retain their transfer/withdrawal path; a conditional escrow requiring an invalidated source checkpoint can remain locked. There is no implemented permanent-halt recovery guarantee. Any proposed recovery mechanism must preserve already-earned payouts and receive separate security review before admission.

## Definition of done

Zutti's Testnet swap is complete only when the live cases above pass using authenticated source history, genuine closed proofs, actual contract verification, actual token backing and real wallet execution. The working demo, public repository and video must show that achieved behavior and explicitly state the SPV, Ethereum bootstrap, privacy and halt assumptions.
