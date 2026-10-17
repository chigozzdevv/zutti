# Zutti

Private cross-chain RFQ swaps between native shielded ZEC and privately owned USDC.

This repository implements Rust protocol components, native Zcash transaction validation, local and Boundless preparation proving, and Ethereum settlement state machines. No Zutti contract is deployed, and end-to-end swaps are not available. The intended protocol and SDK APIs are specified in [zutti.md](zutti.md).

| Component | Implemented now | Remaining boundary |
| --- | --- | --- |
| `protocol` | Exact amounts and fee arithmetic, strict encodings, deployment domains, fixed order terms, private-note openings, operation-bound spending authorization, pool membership trees and settlement journals | Production cryptographic profile, membership/conservation proofs and circuit parameters |
| `wallet` | Pinned Ironwood PCZT inspection, output commitment/ciphertext/memo reconstruction, withheld authorization checks, encrypted recovery archives and private-note encryption/recovery | Chain scanning, production profile admission, verified Ethereum finality and source authorization/broadcast integration |
| `source` | Real-block parsing, transaction/authorizing-data roots, Equihash, expected difficulty, median-time rules, genesis identity, cumulative work, versioned chain history, shielded-tree frontiers, transaction signatures/proofs, UTXO/nullifier/anchor checks, monetary rules, authenticated state access transcripts, permanent header checkpoints, atomic candidate replay and private history-query predicates | Complete consensus-rule audit, current-chain bootstrap evidence, deployed fork/stability admission and durable source storage |
| `prover` | RISC Zero history-query, preparation, recursive source-prefix and source-registry guests, genuine succinct/Groth16 verification, pinned image/journal binding, bounded proof artifacts and witness CLIs | Production profile admission, current-chain bootstrap, private-pool and settlement proofs |
| `boundless` | Capped Base proof requests, public demo transport, durable request records, proof collection and on-chain timing/cost evidence | Confidential witness transport and representative source workloads |
| `contracts` | Actual Groth16 preparation verification; source-update registry and private-pool execution state machines; Rust/Solidity journal agreement | Current-chain bootstrap, pool proof programs, cryptographic settlement composition and deployment |
| `recovery` | Local archive authentication/inspection with private key-file input | Validated swap-record restoration and independent on-chain claim/refund execution |

The SHA-256 commitment/index suite is explicitly a **development profile**. It is not an admitted production circuit suite. Native note ownership and encryption alone do not issue backed USDC. The pool execution contract requires source and pool evidence under its pinned programs. Current-chain source bootstrap and the pool proof guest remain unfinished.

## Development

Required: Rust **1.93.0** with Cargo, Clippy and rustfmt; RISC Zero SDK/server **3.0.5** with the RISC Zero Rust toolchain (this build uses **1.94.1**) and C/C++ toolchain **2024.1.5**; Node **22.13 or later in the 22 series, or 24+**; PNPM **12.3.4**; Foundry. Rust is pinned by `rust-toolchain.toml`; JavaScript and Rust dependency lockfiles are versioned source artifacts.

```sh
pnpm run setup
pnpm build
pnpm lint
pnpm typecheck
pnpm test
```

`pnpm run setup` checks tools and fetches locked dependencies. Use `run` here because bare `pnpm setup` is PNPM's own shell-setup command. Build, lint, typecheck and test work offline after dependencies have been fetched. No wallet credentials, running chain node or funded account are required for these checks. The TypeScript workspace configuration is ready; there are no implemented TypeScript SDKs to typecheck yet. `pnpm typecheck` currently checks the Rust workspace.

`contracts/foundry.toml` pins Solidity 0.8.30 and Cancun. `PreparationVerifier` pins the deployment domain and preparation image, checks the exact journal and calls the upstream Groth16 verifier with a bounded gas allowance. Vendored verifier sources retain their licenses and hashes in `contracts/vendor/provenance.json`. No Zutti contract has been deployed.

Inspect an actual historical block fixture:

```sh
cargo run --locked -p zutti-source -- inspect \
  --network mainnet --format hex \
  --block tests/fixtures/block-1687106.txt
```

The result reports both `work_verified` and `consensus_verified`. Standalone inspection checks Equihash and the claimed target. `HeaderCandidate` separately starts from an exact network genesis and enforces consecutive ancestry, expected difficulty, median-time rules and cumulative work. It includes Blossom spacing and testnet minimum-difficulty rules, checked against upstream reference vectors. Neither path proves transaction/state validity, authenticated history commitments or stable canonical ancestry. `consensus_verified` remains `false`, and `verify_history` still rejects every candidate.

```sh
cargo run --locked -p zutti-source -- status
cargo run --locked -p zutti-recovery -- status
```

The recovery CLI authenticates an encrypted archive locally:

```sh
cargo run --locked -p zutti-recovery -- inspect \
  --backup /absolute/path/swap.backup \
  --key-file /absolute/path/recovery.key \
  --domain DEPLOYMENT_DOMAIN_HEX
```

The key file contains exactly 32 raw bytes, independently generated for recovery encryption. Archive and key files must be regular, owner-only files within trusted parent directories. Secret key bytes are never command-line arguments. Inspection prints authentication status and payload size, not plaintext or transaction IDs. It does not validate an economic claim. `claim`, `refund` and `source-prover` return a nonzero unavailable result. The private prover implements the history-query and payment-effects preparation components described below; neither authorizes settlement.

## Private history-query proofs

`QueryWitness` privately carries the immutable order, reservation openings, transaction ID, complete 256-sibling history path and candidate snapshot. The guest checks the domain/genesis/policy binding, recomputes both commitments and evaluates the inclusive payment window. A qualifying inclusion selects the fixed payout; absence is eligible only after expiry and selects the fixed refund. Waiting and out-of-window inclusion cannot produce a claim.

The journal has one public shape: deployment domain, order/reservation commitments, history context/root/height, selected output commitment and ciphertext digest. It contains no transaction ID, inclusion height, amount, receiver, private opening or paid/refund selector. Counterparties can still recognize their own terms; timing and network metadata remain relevant.

`ProofKind::HistoryQuery` is distinct from `Source` and `Settlement`. A valid query receipt proves this predicate against the supplied root. It **does not** establish that root's complete consensus history, its stable ancestry extending the order anchor, the prepared payment's correctness/expiry, pool membership, note conservation or destination finality. Those checks must be composed before settlement. The current verifier cannot turn this receipt into `VerifiedHistory` or authorize source payment.

The backend uses pinned RISC Zero 3.0.5 guests, local `r0vm` and SHA-256 acceleration with the same native transcript bytes. Verification supports succinct and raw Groth16 receipts, pins the compiled image and exact journal, rejects fake and composite receipts, explicitly disables development-mode verification and bounds receipt decoding. Remote transport is an explicit native command, separate from local private proving. Browser/WASM proving is not implemented. The guest has its own dependency lockfile; its build always uses `--locked` and cannot skip guest compilation.

Run the expensive real-proof checks separately:

```sh
pnpm run test:proofs
```

The tests derive candidate indices from three attributed historical blocks. They test genuine inclusion/non-inclusion queries, not an end-to-end valid Ironwood payment or a consensus-accepted source root. They also exercise encrypted witness loading, proof persistence and independent CLI verification. Routine `pnpm test` covers deterministic predicates, malformed bindings and fake-receipt rejection without generating new proofs.

Each real-proof run retains its public artifacts and timings under `.data/proof-evidence/run-*`. Test witness archives and recovery keys use temporary owner-only files. Private proving trusts the local machine and prover process with the witness.

A caller can generate a component proof from an owner-only encrypted archive containing canonical `QueryWitness` bytes:

```sh
cargo run --locked -p zutti-prover --bin private-prover -- prove-query \
  --backup /absolute/path/query.backup \
  --key-file /absolute/path/recovery.key \
  --domain DEPLOYMENT_DOMAIN_HEX \
  --out /absolute/path/query.proof

cargo run --locked -p zutti-prover --bin private-prover -- verify-query \
  --proof /absolute/path/query.proof
```

The archive uses the existing authenticated backup format and separate 32-byte recovery key. The CLI prints verification status, without private terms or the branch. It always reports `consensus_verified: false` and `settlement_authority: false`. Verification of an artifact's embedded statement establishes that statement only; an application must compare it to its independently expected statement using `ProofArtifact::verify(expected)`.

### Historical history-query evidence

These artifacts were generated for the preceding development image, not the current compiled image. The verifier rejects unregistered images. Both recorded proof paths passed, including altered-journal rejection, changed-claim rejection, proof-kind relabelling rejection and independent CLI verification with development mode enabled. Fake receipts remain rejected. The absence path additionally passed encrypted archive input and verbose-log separation from JSON output.

| Case | Observed time | Public artifact bytes | Evidence |
| --- | ---: | ---: | --- |
| Paid | 256.46 s | 224,256 | [timing](.data/proof-evidence/run-1Rwsszl8/paid.json), [proof](.data/proof-evidence/run-1Rwsszl8/paid.proof) |
| Expired and absent | 277.22 s | 224,256 | [timing](.data/proof-evidence/absence-cli-check/expired_absent.json), [proof](.data/proof-evidence/absence-cli-check/expired_absent.proof) |

Program image: `115e817a997cb003c29a150e77757b7f8a38daee10d9830211a6273e10174195`. Each envelope is 223,885 bytes; the public journal is 274 bytes. These are local test-driver observations including proving and client verification, excluding compilation. The paid case uses the native API; the absence case includes encrypted CLI input and artifact creation. They do not measure full Zcash consensus/bootstrap proving, Ethereum verification or an executed swap. Cross-machine reproducible release images and production profile admission remain deployment work.

## Private preparation proofs

`PreparationWitness` carries the agreed order, commitment openings and upstream Ironwood PCZT. Its preparation payload removes spending keys, full viewing keys, input note openings, signatures and Merkle witnesses. The original wallet PCZT remains separate for signing and recovery. Encrypted archive proving stays local; the separate Boundless demonstration exports generated public test data.

The guest checks the pinned v6/NU6.3 Ironwood-only profile, enabled outputs, the agreed expiry, fee bound and exact transaction-ID digest. It reconstructs the designated V3 note commitment, ephemeral key and authenticated note ciphertext, including the order/anchor memo. The public journal is exactly 106 bytes: version, deployment domain, order commitment and reservation commitment. It exposes no transaction ID, receiver, amount, expiry or commitment openings.

The wallet's full inspector additionally verifies the spend metadata, input/output value balance, randomized authorization keys and withheld real-spend signatures. These local signing checks are separate from the payment-effects statement. The preparation proof does not prove input existence, unspentness, source consensus, future inclusion, private USDC funding or destination finality. Authorization remains unavailable.

```sh
cargo run --locked -p zutti-prover --bin private-prover -- prove-preparation \
  --backup /absolute/path/preparation.backup \
  --key-file /absolute/path/recovery.key \
  --domain DEPLOYMENT_DOMAIN_HEX \
  --out /absolute/path/preparation.proof

cargo run --locked -p zutti-prover --bin private-prover -- verify-preparation \
  --proof /absolute/path/preparation.proof
```

The archive contains canonical `PreparationWitness` bytes. The existing archive transport accepts up to 4 MiB of plaintext; the native witness codec separately bounds PCZT input at 8 MiB. Larger preparation payloads require another explicitly supported private transport. The CLI separates prover logs from its JSON result and rejects a history-query artifact supplied as preparation evidence, or vice versa.

The expensive test suite includes native/guest agreement, wrong-memo rejection and genuine preparation receipt generation. Execution alone is not proof verification. Local execution measured about 62.6 million cycles for a two-action synthetic PCZT after removal of unused input metadata. A genuine local preparation proof and its tampering checks completed in 15,753.99 seconds on this Mac. This is one synthetic workload; production proving latency is not established. History queries use a segment limit of 2^18 cycles; the heavier preparation prover uses 2^20 to reduce segment overhead without changing the statement.

Guest image IDs can change when dependencies or crate layouts are rebuilt. The verifier retains the explicitly pinned first preparation and history-query images under their existing journal semantics, alongside the current component builds. A committed generated Groth16 fixture checks that the first preparation receipt still verifies; unknown programs, proof-kind substitutions and altered seals are rejected. Source-prefix and registry-update programs are implemented. Current-chain bootstrap and private-pool proof programs remain separate work.

## Boundless public demonstration

`crates/boundless` is a separate native workspace pinned to `boundless-market` 2.0.1 and Alloy 1.8.3. This keeps transport dependencies out of the Zcash guest dependency graph. It uses the Base mainnet Boundless market for proving payments; this does not deploy a Zutti pool or execute a mainnet swap.

Configure an owner-only `.data/boundless.env` using the fields in [the configuration example](crates/boundless/.env.example). The price cap is a fixed wei amount, not a floating dollar limit. The client verifies the signer/address and chain, disables automatic funding, refuses to overwrite request records and caps each request before submission. A failed or interrupted submission retains its request ID; inspect or collect that request before attempting another paid job.

```sh
pnpm boundless balance
pnpm run test:boundless
```

The second command **spends the configured proving balance**. It generates a synthetic preparation witness, exports the pinned program/input/expected statement, submits one job, collects the raw Groth16 seal and timing evidence, cryptographically imports it against the locally retained statement, and runs the actual Solidity verifier and tampering checks. Routine `pnpm test` never purchases a proof. The Solidity real-proof test is explicitly skipped unless `ZUTTI_PUBLIC_JOB_DIR` points to retained evidence.

This transport publishes program/input data through Pinata/IPFS and accepts only jobs explicitly marked `public`. **Use generated demo witnesses only.** It does not implement restricted S3 delivery or confidential computation. Encrypting a note does not conceal its opening when that opening is included in a proving witness.

The generated two-action preparation benchmark on 17 October 2026 produced a genuine 260-byte raw Groth16 seal. Rust and the upstream Solidity verifier accepted it; changed order, domain and seal checks rejected it.

| Measurement | Observed result |
| --- | ---: |
| Submission to on-chain fulfillment | 483 s |
| Locked request to fulfillment | 420 s |
| Proving-market fee | 0.000024970337004936 ETH |
| Fee at the sampled ETH/USD quote | About $0.06 |
| Market deposit after this job | 0.001725985294258909 ETH |
| Further jobs at the fixed 0.000062425842512343 ETH cap | 27 |

Capacity excludes gas, pending requests and other PNLX spending from the shared account. The sampled ETH/USD quote was $2,402.85103033. These timings include market waiting, proving, aggregation and on-chain fulfillment; they are not isolated GPU timings or full-source/swap measurements.

Evidence: [on-chain fulfillment](https://basescan.org/tx/0x58cad63b8ff9ec739490d1b00e03321e9877610f750af517037b73249cac6891), [retained timing/cost record](.data/public-preparation-20261017/evidence.json), [verified preparation artifact](.data/public-preparation-20261017/preparation.proof). Program image: `f97aeb93b4066460d372b433a89bd07d6be30934a55fa5de6a5e4c421d9b3916`. The market result alone is not treated as cryptographic verification: import reconstructs the expected receipt claim and independently verifies the Groth16 proof with pinned parameters.

## Native private notes

The `notes` protocol feature defines strict USDC note openings, owner keys, a commitment to a separate nullifier key and canonical operation-bound secp256k1 signatures. A nullifier requires the owner's secret nullifier key, so a sender who knows the note opening cannot derive the future spending nullifier from that opening alone. The signature commits to the exact note, domain and intended operation. A prover can carry that signature without receiving the spending key.

The native wallet encrypts note recovery data using X25519, HKDF-SHA256 and XChaCha20-Poly1305. It authenticates the domain, commitment, ephemeral key, nonce and recipient viewing key. Decryption checks the recovered note against its committed identity. Viewing recovery and spending authorization use separate keys; low-order encryption keys, changed ciphertext/metadata, wrong viewing keys, wrong operations and noncanonical signatures are rejected.

These are native primitives with deterministic encryption available for future proof witnesses. Pool membership paths and contract nullifier consumption are implemented. Their zero-knowledge membership/conservation proofs, correct-output-encryption proofs, production-backed issuance and scanning remain unimplemented. `FixedOutput` binds a ciphertext digest; constructing it alone does not prove that a decoded ciphertext was encrypted correctly.

## Source authorizing-data commitments

The block parser computes version-correct authorizing-data leaves and the ZIP 244 tree with zero padding. The combined block-commitment checker binds that tree to the preceding chain-history MMR root using the specified BLAKE2b personalizations. Differential tests compare the roots and combination with pinned `zebra-chain` 13.0.1, including historical blocks containing legacy transactions. A mutation test changes authorizing data while preserving the header and transaction-ID list, and demonstrates rejection by the combined commitment check.

The preceding MMR root must come from validated authenticated source state. A caller-supplied root does not establish that state. Block inspection reports the computed root and keeps `authorizing_commitment_verified` and `consensus_verified` false until this state is available.

`CandidateHistory` now implements the native chain-history frontier using pinned `zcash_history` 0.5.0: Heartwood/Canopy V1, NU5-era V2 and NU6.3 V3 with separate Ironwood metadata. It starts from the exact network genesis, enforces consecutive parent/height/branch context, checks the preceding history commitment before an upgrade reset, and combines the authorizing-data root from NU5 onward. Snapshots retain at most 32 peaks, reject noncanonical node encodings and validate peak shape, ranges and arithmetic before mutation. Differential tests compare every root across 65 appends per representative epoch with upstream, and compare real-block leaf metadata/counts with Zebra.

The standalone accumulator accepts commitment-tree roots from its caller. `SourceCandidate` instead supplies roots computed by its owned trees as described below. Neither a caller-supplied root nor a decoded candidate snapshot establishes an authenticated bootstrap. The consensus MMR resets at network upgrades; it does not replace Zutti's required history/index commitments for older anchors. Current-chain bootstrap and `VerifiedHistory` remain unavailable; the source prover separately verifies cryptographic chain-prefix evidence.

## Source commitment trees and candidate replay

The block parser retains ordered Sprout, Sapling, Orchard and Ironwood note commitments. `CandidateTrees` derives each pool's root using bounded incremental frontiers; Orchard and Ironwood stay separate. Sprout compression and tree roots are checked against Zebra, and differential tests compare commitment extraction and the resulting roots for the same public-block inputs. These comparisons start from matching empty frontiers; they do not establish the omitted historical prefix before those fixture blocks.

`SourceCandidate` starts from an exact network genesis and atomically advances expected-difficulty/work checks, transaction validation, UTXOs, shielded anchors/nullifiers/value pools, subsidy/funding streams, shielded trees, native chain history and the transaction index. It checks the computed Sapling header root during Sapling/Blossom and the applicable chain-history/authorizing commitment afterward. A failed transition cannot partially advance the index or other state. An index whose configured start is ahead of the processed prefix stays unavailable.

Replay the real mainnet genesis and first block:

```sh
cargo run --locked -p zutti-source -- replay \
  --network mainnet --format hex \
  --genesis tests/fixtures/block-0.txt \
  --block tests/fixtures/block-1.txt \
  --rules 0101010101010101010101010101010101010101010101010101010101010101
```

The rules value is an example index-context identifier, not an admitted consensus profile. Repeat `--block` in consecutive chain order for a longer supplied prefix. The command reports computed public roots and work, rejects skipped ancestry and keeps `consensus_verified` and `settlement_authority` false. Native transaction/spend/monetary checks are connected to replay. Accepted source verification still requires a complete consensus-rule audit, authenticated state witnesses, recursive genesis proofs and the admitted fork/stability policy.

Zebra's standard router retains mandatory historical checkpoint validation even with `checkpoint_sync = false`; this configuration cannot satisfy Zutti's full-prefix requirement. The relevant behavior was checked in `zebra-consensus` 16.0.0, `config.rs` and `router::init_checkpoint_list`. Zutti uses no checkpoint shortcut as verified history. The native verifier includes historical PHGR and Sprout Groth16 verification rather than delegating those transactions to a checkpoint. Recursive prefix proving and authenticated state admission remain necessary.

## Native transaction validation

The verifier implements pre-Overwinter signature hashing, Overwinter, Sapling and v5/v6 signature hashing through pinned upstream primitives, transparent scripts, value conservation, duplicate-input checks, coinbase maturity/shielding, inclusive expiry, lock time and block signature-operation limits. Coinbase heights use the canonical script prefix; pre-v5 coinbases are reparsed with the actual block branch before authorization checks.

Shielded checks verify Sprout PHGR and Groth16 proofs, JoinSplit signatures, individual Sapling spend/output proofs and authorization/binding signatures, and Orchard/Ironwood proofs with the applicable historical or current circuit key. Coinbase shielded outputs must recover with the consensus zero outgoing viewing key and the required note version. Public verifying-key provenance and reproduction are documented in [the parameter directory](crates/source/parameters/README.md).

Spend state separates all four nullifier sets, checks earlier final anchors and Sprout transaction-local intermediate anchors, and accounts for each shielded pool. Pinned Zebra parameters supply subsidy, founders reward, funding streams and lockbox disbursement schedules. Required outputs are counted distinctly; deferred value and NU6's exact coinbase accounting are checked. Persistent ordered maps make candidate clones share state instead of copying the entire ledger each block.

Tests cover published signature vectors, historical PHGR/Groth16 transactions, modern shielded transactions and altered signatures, public coinbases across upgrade boundaries, the first ten consecutive mainnet blocks, and rejection of wrong founders rewards and excessive coinbase value. These are component and prefix tests, not proof that the entire Zcash history has been replayed or proved.

## Settlement contracts

`SourceRegistry` verifies a journal binding the network genesis, policy, previous head, previous stable checkpoint, new state, new checkpoint and increased cumulative work. It retains accepted checkpoints for later recovery. It has no committee signer or administrative root-insertion function. Its full source program must enforce validity, ancestry, bootstrap and fork/stability rules; the registry's work comparison alone does not establish those facts or identify an unseen heavier fork.

`PrivatePool` binds its domain to the destination chain, pool address, token, source policy and proof profiles. Its five operation modes are deposit, transfer, withdrawal, reservation and settlement. Deposits require exact token receipt. Internal operations consume unique nullifiers and append commitments without transferring ERC-20 principal. Reservations also verify the prepared payment and consume an order-use tag. Terminal settlement consumes one reservation nullifier and appends one output; it has no claim timer. Outputs retain public ciphertexts, and old membership roots remain usable with unspent nullifiers.

Rust and Solidity agree on the 295-byte pool journal, source-update journal and deployment-domain vectors. Contract tests cover claim binding, replay, duplicate commitments, token failures, reentrancy and atomic rollback. **Source and pool state-machine tests use `JournalVerifierMock`; their proof programs are not implemented or admitted. Only preparation is tested here with a genuine Groth16 proof.** The contracts are not deployed or ready for funding.

## Ownership

```text
crates/protocol/       canonical protocol types and development transcript suite
crates/wallet/         local Zcash inspection and encrypted archive primitives
crates/source/         public block parsing and candidate history/index processing
crates/prover/         local/remote private component proofs and strict receipt verification
crates/prover/methods/ compiled RISC Zero guest and generated program identity
crates/boundless/      isolated native paid proof transport and public demo evidence
crates/recovery/       independent native archive inspection CLI
tests/fixtures/       cross-language vectors and attributed public Zcash blocks
tests/support/        shared integration-test constructors, never production defaults
scripts/              root shell automation
contracts/            pinned preparation adapter and upstream Groth16 verifier
```

The next TypeScript phase follows `client/src/app`, `client/src/components`, `client/src/shared`, `server/src/features`, `server/src/shared`, and `packages/{sdk,maker-sdk,runtime,schema,bindings}`. Public server processes must not hold maker spending authority or private proving witnesses. Package code must not import server internals.

Root scripts coordinate tools. Package-owned generation stays in `packages/bindings/codegen.ts` once bindings exist. Rust modules use standard Rust naming; TypeScript files use kebab-case and the agreed server feature convention.

## What the tests establish

- Rust and Node independently reproduce fixed encoding/transcript vectors and exact integer values.
- Ironwood fixtures use the real upstream PCZT builder, note encryption, commitment checks, transaction digest and signer. They are synthetic local-consensus notes, not mined payments. Tests show that signing preserves the bound transaction ID and that a wrong memo, receiver, authorization state or excessive source fee is rejected.
- Historical public block fixtures cover both network genesis identities, pre-upgrade and post-NU5 transaction parsing, Equihash and claimed work targets. Their source revision, license and file hashes are retained in `tests/fixtures/provenance.json`. These fixtures are test input, not checkpoints accepted by the protocol.
- Candidate index tests cover all 256 transaction-ID bits, first-inclusion retention, inclusive expiry, delayed paid observations, truncated/tampered paths and atomic batch rejection. Candidate absence is **not** a refund proof.
- Recovery tests cover authenticated encryption, nonce freshness, altered ciphertext/header rejection, bounded reads, owner-only permissions, symlinks and refusal to overwrite existing recovery files. The archive format currently retains caller-provided private bytes; it does not yet define a complete recoverable swap transcript.

## Next verification milestone

Connect complete Zcash consensus and authenticated state transitions to the proving backend, including a reproducible genesis prefix. Implement the source proof program that enforces the registry fork/stability policy. Only then can the candidate index support actual paid/refund evidence. Production preparation-profile admission, private pool/reservation/settlement proofs, verified Ethereum finality and independent recovery remain necessary. Boundless supplies proving capacity for implemented programs; it does not fill these missing statements.

References: [PCZT 0.9.3](https://docs.rs/pczt/0.9.3/pczt/struct.Pczt.html), [ZIP 229](https://zips.z.cash/zip-0229), [ZIP 244](https://zips.z.cash/zip-0244), [ZIP 203](https://zips.z.cash/zip-0203).

Proof backend references: [RISC Zero 3.0.5 receipts](https://github.com/risc0/risc0/blob/v3.0.5/risc0/zkvm/src/receipt.rs), [RISC Zero 3.0.5 SHA-256 API](https://docs.rs/risc0-zkvm/3.0.5/risc0_zkvm/sha/index.html).

Source commitment references: [ZIP 244 block-header commitments](https://zips.z.cash/zip-0244#block-header-changes), [Zebra consensus 16.0.0 configuration](https://docs.rs/zebra-consensus/16.0.0/src/zebra_consensus/config.rs.html), [Zebra consensus 16.0.0 router](https://docs.rs/zebra-consensus/16.0.0/src/zebra_consensus/router.rs.html).

## Authenticated source transitions

`SourceCandidate::record_transition` produces bounded state-access transcripts for a batch of consecutive blocks. `TransitionWitness::evaluate` runs the same transaction, spend, monetary, header and history validation against Merkle-authenticated UTXO and shielded-state accesses. It also verifies every transaction-index update and permanent header-archive insertion. Missing accesses, unused accesses, wrong keys, altered preimages, inconsistent roots and malformed encodings fail. The witness is anchored by the prior `StateSnapshot` commitment; a decoded snapshot alone never establishes a valid prefix.

The permanent archive retains each height's header hash, cumulative work and complete transaction-index root across Zcash history-tree resets. `SourceCheckpoint` selects an index snapshot behind the candidate head and authenticates earlier order anchors. `SourceUpdateWitness` requires increasing proved work and preserves the previous accepted stable header. Its native evaluator checks these predicates; the guest must additionally verify the required chain and predecessor receipts before the result can enter `SourceRegistry`.

Tests replay real mainnet blocks 0 through 10 with different batch partitions and index start heights, compare native and stateless results, and reject forged state and ancestry. This evidence does not demonstrate a genesis-to-current-chain proof or an end-to-end swap.

## Recursive source proving

The separate source guest validates the exact configured genesis and continues from committed source-state snapshots. A continuation carries a raw Groth16 predecessor seal, verifies it inside the VM under the policy's source program, and runs the authenticated transition. This composition works with Boundless's stdin transport. Source policies bind both source and registry program images, network, index start and confirmation depth.

`ProofKind::SourceTransition` proves a chain prefix. The registry guest additionally verifies that proof, selects the stable header/index through the permanent archive, and preserves the previous accepted stable header while requiring increasing work. Its journal matches `SourceRegistry`. The `verify_history` convenience API still rejects candidate snapshots. Source receipts are verified through `ProofVerifier`, and destination admission is handled by `SourceRegistry`. A component receipt does not establish that a chain head is current.

```sh
cargo run --locked -p zutti-prover --bin source-prover -- status
cargo run --locked -p zutti-prover --bin source-prover -- genesis   --network mainnet --genesis tests/fixtures/block-0.txt   --confirmations 3 --out .data/source-genesis
cargo run --locked -p zutti-prover --bin source-prover -- execute   --job .data/source-genesis
```

These commands export public chain data and measure execution. Submission and collection use the existing capped Boundless client; `private-prover import-boundless` verifies the returned seal against the retained statement. `source-prover continue` takes the predecessor proof, exact native prefix and consecutive new blocks. `source-prover update` produces the registry proof job and checkpoint package. The command help lists the complete arguments. Execution output always distinguishes execution from proof generation.

The first mainnet-genesis job executed in 24,111,161 cycles after replacing repeated empty-tree construction with 66 consensus constants independently checked against the pinned upstream implementations. The preceding implementation used 1,056,212,137 cycles. Boundless fulfilled the genuine genesis proof in 76 seconds after submission; it cost 35,790,816,373,743 wei from the proving deposit. Local cryptographic verification passed. The retained public fixture tests verification and altered-seal rejection.

A continuation through mainnet blocks 1 to 4, including in-guest verification of the genuine genesis proof, executed in 253,879,014 cycles. These measurements cover this early public prefix. They do not establish current-chain proving cost, a current bootstrap or a completed swap.

The source registry enforces an immutable, nonzero `minimumWork` as well as increasing head work. A deployment must choose a floor appropriate to its source network before accepting liquidity. The local script uses `1` only behind a chain-31337 guard. This work floor does not replace full chain validation or assert a trusted state root. The registry compares proved histories; learning the heaviest available history still depends on permissionless relayers, data availability and the source chain's hash-power assumptions. A reorganization past the accepted stable prefix halts incompatible updates.

`source-update-proof.json` is genuine public registry-update evidence for the early mainnet prefix. The default Solidity test verifies it with the upstream Groth16 verifier and checks changed state, altered seal, replay and bootstrap-floor rejection. This fixture is not a current-chain bootstrap.
