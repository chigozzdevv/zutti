# Source-verification review

Reviewed 2026-10-24. Scope: public Zcash Testnet and Ethereum Sepolia.

## Decision

Full-history proving with the current RISC Zero source program is paused. Its [execution model](../benchmarks/validation/MODEL.md) does not establish a practical bootstrap. Increasing job limits or purchasing more compute is not an approved remedy.

On 2026-10-23, the user explicitly approved building the SPV prototype with the mining-validity assumption. Full historical transaction-validity proving is no longer the prototype target. No replacement deployment policy is admitted; existing verifier identities and settlement gates remain unchanged. See the [prototype and verification commands](../benchmarks/spv/README.md).

The recommended candidate is **deterministic proof-of-work header verification plus complete, bounded payment-history proofs**. It verifies chain linkage, actual difficulty rules and Equihash, while relying on the Zcash mining network to enforce transaction validity. It preserves the no-custodian and no-checkpoint-committee constraints. It does not preserve independent proof of every historical transaction and state transition.

This is an architecture recommendation, not a demonstrated affordable bootstrap or a working swap. No candidate reviewed here has demonstrated the complete original requirements within the available resources.

## Implemented prototype

The [isolated SPV program and harness](../benchmarks/spv/README.md) implement genesis-bound header transitions, complete bounded inclusion/absence and source-admission statements. Source and admission guests execute using a genuine genesis predecessor; adversarial absence/coverage/parent cases fail. The retained version-2 receipt authenticates genesis only. [Source evidence](../benchmarks/spv/results/2026-10-23-admission/manifest.json)

The separate [payment-window binding prototype](payment-window-binding.md) checks the exact prepared Ironwood output, encrypted anchor memo and expiry, connecting the existing reservation to the SPV window reservation. Its guest execution accepts generated public payment effects and rejects five altered cases. That earlier milestone recorded 38 passing Rust tests. No genuine binding receipt was generated. Its proposed lower bound depends on the explicit no-feasible-commitment-cycle assumption described below; independent review and accepted source ancestry remain required. [Binding evidence](../benchmarks/spv/results/2026-10-24-binding/manifest.json)

The separate, undeployed `SpvRegistry` compares work, observes competing candidates, checks freshness and the maximum timestamp across the prefix, advances only the accepted stable ancestry, and can halt on proved heavier stable-history conflicts. Ten SPV contract tests use mock verification; their journal bytes match Rust guest execution. A genuine admission receipt and Ethereum wrapping have not been demonstrated. No trust committee or privileged checkpoint setter is introduced.

The [settlement and recovery milestone](spv-settlement.md) implements backed reservation, fixed-output terminal composition, Ethereum authorization and encrypted independent witness preparation. Its [coverage evidence](../benchmarks/spv/results/2026-10-24-coverage/manifest.json) records bounded recursive processing for dense refund intervals. Conditional cases are execution-tested with unresolved proof assumptions; no current-chain or funded-swap claim follows.

The window program directly checks complete block bodies; the reusable public interval index described below is not implemented. Its version-2 journal binds the exact ending checkpoint, ready for explicit admission composition. It cannot establish the payment's lower bound by itself. Binding/source receipt composition, review of the lower-bound construction and funded Ethereum settlement remain outside its authority. Existing deployment-policy gates have not been relaxed.

## Required behavior

The roles are the ZEC payer and the USDC funder; either can be the RFQ maker.

Before payment, they agree one exact prepared shielded transaction, a finite source window, fixed private-USDC payout/refund notes and recovery data. The transaction remains unauthorized until its matching USDC reservation is final under the accepted Ethereum policy.

Settlement must establish one of two facts:

- **Paid:** that exact transaction occurred within its permitted window on the accepted stable source ancestry. Its fixed private-USDC payout remains claimable without a destination claim deadline.
- **Unpaid:** that exact transaction is absent from every possible inclusion block through its inclusive expiry, and the accepted stable source history has passed expiry. Only then can the fixed USDC refund be created.

Both outcomes consume the same escrow nullifier. Nullifier uniqueness prevents two terminal spends; source evidence must also ensure that the permitted branch is correct.

Zcash transaction expiry prevents first inclusion after the expiry height. It does not establish absence before that height. [ZIP 203](https://zips.z.cash/zip-0203)

## What can change the workload structurally

| Approach | Source evidence | Additional assumption or limitation | Decision |
| --- | --- | --- | --- |
| Current full-validity zkVM | Every consensus transition from genesis | Present execution cost; no practical complete bootstrap demonstrated | Pause |
| Reusable full-validity receipts | Independently verified receipts for the required network, rules and ancestry | A matching available receipt chain is needed; a node database is insufficient | No usable current-chain receipt chain established by this review |
| Accelerated full-validity verifier | Same validity statement with cheaper cryptography and composition | Substantial engineering; no measured complete speedup yet | Separate research; no delivery commitment |
| Deterministic SPV and bounded history | Every header transition, then complete transaction enumeration only where settlement needs it | Miners enforce transaction/state validity; relevant chain evidence must reach the verifier | Approved prototype; deployment admission remains pending |
| FlyClient and bounded history | Sampled history plus deterministic recent checks and complete refund coverage | Sampling parameters, difficulty analysis, epoch transitions and a complete verifier | Not a drop-in replacement |
| Bilaterally agreed recent anchor | Proofs after an anchor independently checked by both parties | Clients must authenticate the anchor; Ethereum does not prove its prefix | Not admitted; this changes bootstrap trust and client duties |
| Optimistic validation | Native validation with a permissionless dispute mechanism | A timely, funded honest challenger and a working dispute system | Not selected; a separate architecture and assumption change |
| Adaptor-signature swap | Cryptographic coordination of both spend paths | A verified Zcash-compatible lock/reveal/refund construction | No ready replacement established |

SPV is an established payment-verification model. Its dependence on honest network validation is explicit in section 8 of the [Bitcoin paper](https://bitcoin.org/bitcoin.pdf). It is a different security profile from full independent validation.

Fault proofs can move work out of routine validity proving, but require a program, an instruction verifier and a dispute game. Existing OP Stack infrastructure does not provide a Zcash transition verifier merely by being imported. [OP Stack specification](https://specs.optimism.io/fault-proof/index.html)

The reviewed [ZSwap proposal](https://forum.zcashcommunity.com/t/zswap-a-shielded-atomic-swap-specification-for-zcash-grant-application-request-for-review/57884) identifies cross-curve compatibility and lock/reveal ordering as work requiring scrutiny. It does not justify treating adaptor signatures as a completed Zutti recovery system.

## Recommended candidate

### 1. Authenticate the source chain

The baseline starts from the fixed Testnet genesis, not a server-selected recent root. A separate source-header program must verify consecutive headers, Equihash solutions, header hashes against targets, exact expected difficulty, applicable timestamp rules, network upgrades and cumulative work.

All headers still need processing in this baseline. Removing historical transaction authorization does not eliminate header bootstrap, recursive proving or Ethereum verification costs. Complete bootstrap, genuine non-genesis proving and Ethereum wrapping costs remain unmeasured. Existing downloaded blocks can supply inputs without a new chain download.

Permissionless submissions must expose competing eligible chains. The registry needs an explicit work comparison, observation/maturity period, confirmation policy, freshness rule and stable-prefix rule. A proof cannot establish that an unseen higher-work fork does not exist. Source finality remains probabilistic.

Settlement must stop when the supported rules, source freshness or stable ancestry cannot be established. A longer header chain with incorrect difficulty is invalid. A stale accepted prefix cannot authorize an unpaid refund for an uncovered window.

### 2. Prove the relevant transaction history completely

The public history program parses actual complete blocks in the required interval, derives their transaction identifiers, reconstructs their transaction commitments and links them to authenticated headers. It constructs an authenticated index for that interval. Public index construction can be shared across swaps.

This program does not verify every historical shielded proof, signature or ledger transition. The mining-validity assumption is essential to that omission. Transaction identifiers alone do not establish transaction validity: the modern transaction-ID format excludes validity attestations. [ZIP 244](https://zips.z.cash/zip-0244)

Completeness cannot be obtained from an arbitrary list of 32-byte values. The proof must bind transaction parsing, list boundaries, canonical tree shape, duplicate/padding behavior and applicable block commitments. A collapsed Merkle subtree must not be accepted as a fabricated transaction. Skipping one block or one genuine transaction must fail.

The lower window boundary also needs justification. The [implemented binding prototype](payment-window-binding.md) reconstructs the encrypted memo containing the exact anchor hash and derives the effect-committing transaction ID. If that payment occurred in the anchor's ancestry, the composed encryption, transaction and block commitments would form a dependency cycle. The candidate assumes that constructing this cycle or a different opening of the same committed output is infeasible. This is a conditional cryptographic argument, not a consensus creation timestamp or a theorem established by collision resistance alone. Mutation tests do not prove that assumption. Independent cryptographic review and genuine binding/source receipt composition remain necessary before this boundary can authorize refunds.

### 3. Prove payment or absence privately

A private settlement proof opens the hidden transaction commitment against the public interval index. Inclusion selects the fixed payout. Absence requires complete coverage through expiry and a stable descendant beyond expiry. Any index segments must be consecutive, share the accepted ancestry and cover the boundary exactly.

For example, with an established lower boundary at height 100 and expiry 140:

- A payment at 139 permits payout even when its claim proof arrives much later.
- A refund needs complete absence over 101 through 140 and accepted stable source progress beyond 140.
- Absence through 139 is insufficient.
- A source prefix ending at 140 is insufficient for the current refund predicate.

The numbers illustrate predicates; they are not proposed Testnet confirmation or expiry settings.

### 4. Keep the privacy and recovery obligations

The source payment stays shielded. Destination settlement must hide the transaction-to-escrow link and private note ownership using actual zero-knowledge proofs and hiding commitments. Public source roots do not need to identify which transaction a swap concerns.

The selected maker knows the negotiated deal. Timing, network queries, fees, public deposits/withdrawals and activity patterns can still leak information. A hosted private prover learns its plaintext witness unless a separately validated confidentiality mechanism protects it. Public source proving and private settlement proving therefore have different input-handling requirements.

Each party retains the data required to prove its own terminal outcome. Website failure or maker disappearance must not change that entitlement. Missing source/index data or unavailable proving can still lock funds; a timer must not convert those availability failures into permission to refund a payment already made.

## What the external work actually establishes

### Skyhook

Skyhook describes transaction-ID reconstruction and light-client verification. Its published lifecycle permits a refund after `refundTime` if no claim has been made. Under that documented rule, safe payment requires successful claim delivery before the refund opportunity. Zutti's requested delayed-claim protection needs a stronger refund predicate. This is a comparison of the published rules, not a tested exploit of a deployed contract. [Skyhook lifecycle](https://skyhook-proposal.strahn.dev/contracts/life-of-a-swap/)

Its chain-identity documentation also describes a deployment-selected trusted starting block and a difficulty floor, with participants checking real difficulty off-chain. Those choices cannot be silently substituted for Zutti's genesis-bound, exact-difficulty requirement. [Skyhook chain identity](https://skyhook-proposal.strahn.dev/chain-finality/proving-chain-identity/)

Complete absence-based recovery is a concrete target for improvement. Neither the comparison nor choosing a different implementation establishes novelty or a finished advantage.

### FlyClient

ZIP 221 supplies history commitments. Its security discussion warns that the original analysis does not settle rapidly adjusting difficulty and calls for further Zcash-specific analysis. [ZIP 221](https://zips.z.cash/zip-0221#security-and-privacy-considerations)

The April 2026 [Catching the Fly paper](https://arxiv.org/html/2604.26736v1) is a relevant implementation lead. It supplies a modified Zebra data provider, a work-budget adversary model, parameter analysis and proof-size experiments. Those results are not full transaction-validity proofs or Zutti payout/refund benchmarks.

The associated [Python prototype](https://github.com/Metalcape/zcash-flyclient) was inspected at commit `8b316ef71207f8af92ba7d81a990283b5eaa0600`. Its README marks non-interactive Fiat-Shamir proving unfinished. In `demo.py`, `verify_non_interactive` checks supplied block/proof collections but does not regenerate and enforce the complete required sample set from the committed tip. Its shown checks also do not implement the complete difficulty-transition and header-work validation required by the proposed Zutti profile. This is a scoped code review, not a complete audit or a reproduced live attack. [Reviewed verifier](https://github.com/Metalcape/zcash-flyclient/blob/8b316ef71207f8af92ba7d81a990283b5eaa0600/demo.py#L321-L381)

Sampling source headers would not sample away the refund obligation. Every possible payment-inclusion block in the bounded refund interval still needs authenticated complete coverage.

## Acceptance gates

Implementation of the SPV prototype is approved under the stated assumptions. The existing full-validity profile must not accept its receipts, roots or state snapshots. Prototype approval is not deployment admission.

Before a bootstrap or deployment is authorized, the replacement must demonstrate:

1. Real Testnet header transitions with incorrect Equihash, target, difficulty, timestamp, network and parent rejection.
2. A defined genesis bootstrap and fork/stability state machine with competing-history tests.
3. Real-block paid inclusion and complete expired absence, bound to the same prepared payment and ancestry.
4. Rejection of an omitted transaction, collapsed subtree, missing interval block, wrong epoch/branch and payment included exactly at expiry.
5. Rejection of refund after an actual payment whose destination claim was deliberately delayed.
6. A justified lower search boundary and correct behavior across relevant transaction formats and upgrades.
7. Genuine source and private-settlement proofs, local EVM verification and independent recovery using saved data.
8. Measured execution, genuine proving, aggregation, wrapping, memory, bootstrap cost and ongoing throughput. Historical full-validity cycle estimates must not be reused as a quote for this program.

Testnet demonstrates behavior under its own consensus and available work. It does not establish the economic attack resistance of a production deployment.

If these gates do not pass within the available resources, the candidate is not a practical delivery plan. There is no approved fallback to trusted operators, timeout-only refunds, fabricated proofs or an undisclosed weaker source policy.
