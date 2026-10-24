# SPV reservation, settlement and recovery

This is an isolated Testnet construction. No deployment profile admits these programs, no SPV pool is deployed, and no funded cross-chain swap has been demonstrated. The SPV mining-validity assumption remains explicit. The [payment-window lower-bound argument](payment-window-binding.md) is conditional and has not received independent cryptographic review.

The [Testnet delivery plan](testnet-delivery.md) fixes the remaining work and acceptance evidence. The [refund review packet](refund-window-review.md) separates the proposed cryptographic claim from what the existing tests establish.

## One reservation, two fixed outcomes

The wallet prepares an exact Ironwood payment before USDC is reserved. Its binding claim connects the private order and transaction ID to the ordinary reservation commitment and the SPV window reservation commitment. Both fixed private-USDC notes are part of the order. A quote deadline is not the Zcash transaction expiry or a payout-claim deadline.

`SpvReservation` checks the domain, exact source policy, anchor height/hash, binding claim, escrow amount and both encrypted output notes. The reservation operation consumes authorized private inventory notes and conserves their total value into the conditional escrow plus change. Its source checkpoint is the exact anchor-prefix identity.

`SpvPoolWitness::assumptions()` requires the binding-program receipt and the source-header receipt for funding. For settlement it requires the verified reservation receipt and a source-window receipt. The reservation certificate already composes the binding and anchor proofs, so terminal settlement does not require the original preparation witness or a second copy of its binding receipt. The guest verifies each exact image and journal with `env::verify`; a serialized claim or a native predicate result is not a receipt.

The terminal operation additionally checks:

- The funding certificate uses the same pool program, source policy, binding program, expiry, anchor, order and reservation. Its output digest contains this exact escrow and the original change outputs.
- The window's policy and salted reservation match the binding.
- The window starts from the reservation's exact anchor prefix.
- The operation names the exact ending checkpoint from the source observation.
- The escrow exists in the specified pool membership tree.
- The output is the precise payout or refund note selected by the source proof, including its ciphertext digest and amount.
- The entire escrow value is conserved into that one output.
- Both possible outcomes consume the same escrow nullifier.

The direct source program establishes inclusion within the permitted interval, or complete absence through inclusive expiry with the required descendants. Recursive coverage processes dense refund intervals in bounded complete-block chunks, carrying the same terms, salted target commitment, anchor and any earlier payment into the next proof. Its final receipt uses the same window journal. A partial coverage claim never authorizes settlement. The pool program consumes that evidence; it does not accept a caller's assertion that the payment is absent. Inclusion remains eligible when presented after expiry. No destination timer creates refund authority.

## Contract enforcement

`SpvPool` is separate from `PrivatePool` and uses a new pool-program identity. `ZUTTISPC` version 2 wraps the pool operation journal with the exact source-policy identity, binding-program identity, pool-program identity and funding expiry. The contract pins that pool identity to its immutable verifier image; a caller cannot make settlement trust a different reservation program. Old pool journals cannot be substituted. Note spending signatures authorize this complete new journal.

For conditional operations the contract requires the exact source checkpoint to be accepted by `SpvRegistry`. Reservations require the supported source horizon to extend through expiry plus twice the confirmation depth: the window receipt advances through the first confirmation suffix and registry admission confirms that ending prefix again. This conservative composition adds confirmations; it is not a measured swap-latency estimate.

Funding also requires fresh source admission and at least `minimumRemaining` blocks after the recorded observation head: stable height plus the registry's confirmation depth. This is a check against observed progress, not a guarantee that no unseen newer chain exists or that a transaction will be mined in time.

After proof verification the contract atomically spends the nullifiers, rejects reused order tags and commitments, and inserts the encrypted outputs. Deposits increase backing only after an exact token transfer. Withdrawals reduce backing and deliver tokens to the proof-bound public destination. Reserving or settling private inventory does not transfer ERC-20 tokens to a private key; it creates a private note inside the backed pool. The owner later spends that note or withdraws through a proven operation.

There is no checkpoint setter, discretionary payout selector or administrator refund. A source conflict halt invalidates accepted-checkpoint lookups. It does not undo an earlier payout, and recovery from a permanent source halt is not solved by this implementation.

## Privacy

The public journal omits the Zcash transaction ID, recipient, ZEC amount, memo and explicit paid/refund selector. Funding expiry is public in this prototype so the contract can enforce remaining inclusion time. Pool roots, source checkpoints, note commitments and nullifiers are public. ERC-20 deposits and withdrawals expose their public transfer values and destinations. The selected maker knows its negotiated deal; timing and network correlation remain possible.

Private binding, intermediate coverage and pool dependencies must use genuine succinct or Groth16 receipts. Intermediate coverage diagnostics redact the private payment-presence state. The host rejects fabricated receipts and private composite receipts. Public header-prefix evidence may use a composite receipt. Development-mode verification is disabled.

A prover sees its witness. The Testnet demo permits publishing its swap witnesses through Boundless/IPFS, including inventory-note openings, nullifier material and operation-bound authorizations. Those demo inputs are public and must not be represented as confidential from provers or other input readers. Wallet signing keys remain local. Confidential prover transport is deferred.

## Recovery and payment authorization

`SpvSwapRecord` retains the fixed reservation terms, both output recovery records, anchor and binding claim. It excludes the maker's funding-note spending material. It retains the funding journal and public output bindings and can cache its 260-byte Ethereum proof seal. Seal bytes are not treated as verified merely because they were saved. `recover_funding` reconstructs and cryptographically verifies the receipt under the pinned pool image and exact funding journal; invalid, legacy or substituted seals fail. The public seal can be recovered from the successful funding transaction calldata.

The record can reconstruct the exact terminal witness from a pool path and source observation without the original maker service. The reconstructed witness still needs genuine proof dependencies, an accepted source checkpoint and on-chain submission.

The recovery CLI provides `spv-export`, `spv-restore` and `spv-prepare`. The latter selects the fixed output from the source observation; it does not accept a caller-selected refund branch. It reads authenticated encrypted records and evidence and writes an encrypted witness without broadcasting or claiming proof authority. The independent CLI tests remove the original funding file before preparing both outcomes.

Native records are authenticated and encrypted before persistence. Reads require the expected domain and swap ID. Writes do not replace existing records. A recovery record does not establish that an escrow was funded.

`SpvPaymentJournal` reuses the existing encrypted signing, finalized-transaction and exact-byte retry state machine through a sealed record interface. The legacy journal retains its existing record type and format. Each SPV journal first checks that its PCZT matches the saved reservation's exact transaction ID and payment effects.

The separate Ethereum authorization adapter requires the funding seal to be retained in the encrypted payment record before signing. It verifies that saved reservation receipt and checks the locally retained accepted terms and expected private-USDC receiving key. It also verifies a genuine source prefix extending the saved anchor, source freshness, remaining inclusion time and finalized Ethereum account/storage evidence for the exact escrow, used order tag and unspent terminal nullifier. Its configured pool code hash and chain profile must come from a locally admitted deployment manifest. The new pool's storage slots are checked against those used by the Ethereum verifier. The Ethereum verifier also requires a locally pinned bootstrap checkpoint and bounded checkpoint/finality ages; these are part of its consensus profile. No live provider or deployment profile is configured by this work.

## Evidence and unfinished obligations

The [retained evidence](../crates/prover/spv/evidence.json) distinguishes execution samples from genuine proofs and identifies the programs each result covers. The positive conditional execution cases use unresolved assumptions and synthetic source observations. Negative guest cases check the exact missing dependency digest. They are not funded swaps or cryptographic payment proofs.

| Obligation | Current boundary |
| --- | --- |
| Prepared-payment binding | Implemented and execution-tested; genuine binding receipt and independent lower-bound review remain |
| Backed reservation and fixed terminal outputs | Native predicates, isolated guest and contract enforcement implemented; genuine conditional composition remains |
| Ethereum authorization | Adapter and existing finality/storage verifier; live Sepolia evidence and deployment admission remain |
| Source history | Genesis-bound SPV program; current-chain proof bootstrap and ongoing proving remain |
| Complete dense refund windows | Recursive complete-block coverage implemented and execution-tested; genuine recursive receipts and dense-window proving cost remain |
| Independent recovery | Encrypted records, funding-seal import and standalone witness preparation; genuine SPV funding/window proof collection and live submission remain |
| Source halt | Stops new source-dependent execution; safe recovery from a permanent halt remains unresolved |
| Performance | Bounded local execution measured; complete proof time, wrapping, bootstrap cost and sustained throughput remain unestablished |

These obligations cannot be replaced by a timeout, fabricated proof, trusted lookup server or operator-selected recent source root. Missing evidence blocks settlement. It does not change which party is entitled to the money.
