# Zutti

Private cross-chain RFQ swaps between native shielded ZEC and privately owned USDC.

This repository implements Rust protocol components, native Zcash transaction validation, local and Boundless preparation proving, and Ethereum settlement state machines. No Zutti contract is deployed, and end-to-end swaps are not available. The intended protocol and SDK APIs are specified in [zutti.md](zutti.md).

| Component | Implemented now | Remaining boundary |
| --- | --- | --- |
| `protocol` | Exact amounts and fee arithmetic, strict encodings, deployment domains, fixed order terms, private-note openings, operation-bound spending authorization, pool membership trees and settlement journals | Production profile review and circuit parameters |
| `wallet` | Pinned Ironwood PCZT inspection, output commitment/ciphertext/memo reconstruction, withheld authorization checks, encrypted recovery archives and private-note encryption/recovery | Chain scanning, production profile admission, live finality/reservation evidence and source authorization/broadcast integration |
| `wallet/ethereum` | Pinned Helios beacon light-client verification, committee transitions, finalized execution-header binding and EIP-1186 account/storage proofs for reserved escrows | Trusted Ethereum bootstrap profile, live reservation evidence and wallet signing integration |
| `source` | Real-block parsing, transaction/authorizing-data roots, Equihash, expected difficulty, median-time rules, genesis identity, cumulative work, versioned chain history, shielded-tree frontiers, transaction signatures/proofs, UTXO/nullifier/anchor checks, monetary rules, authenticated state access transcripts, permanent header checkpoints, atomic candidate replay and private history-query predicates | Complete consensus-rule audit, current-chain bootstrap evidence, deployed fork/stability admission and current-chain storage/throughput measurements |
| `prover` | RISC Zero history-query, preparation, private-pool, recursive source-prefix and source-registry guests, genuine succinct/Groth16 verification, pinned image/journal binding, bounded proof artifacts and witness CLIs | Production profile admission, current-chain bootstrap, cross-chain reservation/settlement integration and independent recovery |
| `boundless` | Capped Base proof requests, public demo transport, durable request records, proof collection and on-chain timing/cost evidence | Confidential witness transport and representative source workloads |
| `contracts` | Actual Groth16 preparation verification; source-update registry and private-pool execution state machines; Rust/Solidity journal agreement | Current-chain bootstrap, cross-chain composition, finality and network deployment |
| `recovery` | Local archive authentication/inspection with private key-file input | Validated swap-record restoration and independent on-chain claim/refund execution |

The SHA-256 commitment/index suite is explicitly a **development profile**. It is not an admitted production circuit suite. Native note ownership and encryption alone do not issue backed USDC. The pool execution contract requires source and pool evidence under its pinned programs. Current-chain source bootstrap, cross-chain composition and verified Ethereum finality remain unfinished.

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

The key file contains exactly 32 raw bytes, independently generated for recovery encryption. Archive and key files must be regular, owner-only files within trusted parent directories. Secret key bytes are never command-line arguments. Inspection prints authentication status and payload size, not plaintext or transaction IDs. It does not validate an economic claim. `claim` and `refund` return a nonzero unavailable result. `source-prover` implements prefix proving, registry updates and profile-bound source admission. The private prover implements the history-query and payment-effects preparation components described below; neither authorizes settlement.

## Authenticated source storage

Source UTXOs, shielded state, permanent header records and the transaction index use a persistent compressed sparse tree. A nonempty map with `n` keys retains `2n - 1` tree nodes, sharing unchanged branches across candidate clones. Unary paths and empty siblings are reconstructed when needed. The original 256-level roots and proof encodings remain unchanged; this is a storage change, not a weaker verification rule.

Differential tests compare every sibling and root against an uncompressed reference through inserts, overwrites, deletions, absent keys, adjacent keys and forks. Native replay of the first four mainnet blocks must also reproduce the state, archive and index committed by the retained genuine prefix receipt. Durable disk storage and a current-chain bootstrap remain separate requirements.

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

The guest checks the pinned v6 Ironwood-only profile, enabled outputs, the agreed expiry, fee bound and exact transaction-ID digest. It derives the NU6.3 or NU7 branch from the source genesis and the inclusion window, rejecting unknown networks and windows crossing an upgrade. It reconstructs the designated V3 note commitment, ephemeral key and authenticated note ciphertext, including the order/anchor memo. The public journal is exactly 106 bytes: version, deployment domain, order commitment and reservation commitment. It exposes no transaction ID, receiver, amount, expiry or commitment openings.

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

Guest image IDs can change when dependencies or crate layouts are rebuilt. The verifier retains the explicitly pinned first preparation and history-query images under their existing journal semantics, alongside the current component builds. A committed generated Groth16 fixture checks that the first preparation receipt still verifies; unknown programs, proof-kind substitutions and altered seals are rejected. Source-prefix and registry-update programs are implemented. Current-chain bootstrap and cross-chain reservation/settlement integration remain separate work.

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

The rules value is an example index-context identifier, not an admitted consensus profile. Repeat `--block` in consecutive chain order for a longer supplied prefix. The command reports computed public roots and work, rejects skipped ancestry and keeps `consensus_verified` and `settlement_authority` false. Native transaction/spend/monetary checks are connected to replay. Authenticated state witnesses and recursive prefix proofs are implemented. Current-chain use still requires a complete consensus-rule audit, current bootstrap evidence and the intended fork/stability profile.

Zebra's standard router retains mandatory historical checkpoint validation even with `checkpoint_sync = false`; this configuration cannot satisfy Zutti's full-prefix requirement. The relevant behavior was checked in `zebra-consensus` 16.0.0, `config.rs` and `router::init_checkpoint_list`. Zutti uses no checkpoint shortcut as verified history. The native verifier includes historical PHGR and Sprout Groth16 verification rather than delegating those transactions to a checkpoint. The recursive source guest verifies that complete prefix and its authenticated state transitions. The historical demonstration does not establish the current chain.

## Retained proof programs

A deployed contract pins its proof program. Keep the matching program files with its deployment and recovery material so rebuilding the SDK cannot silently select a different program.

```sh
cargo run --locked -p zutti-prover --bin private-prover -- \
  export-programs --out /absolute/path/deployment-programs

cargo run --locked -p zutti-prover --bin private-prover -- \
  inspect-program --kind pool \
  --program /absolute/path/deployment-programs/pool/program.elf
```

These exports contain public programs and image manifests, without private witnesses or keys. Loading a program recomputes its zkVM image and rejects unknown images or the wrong program kind. Source programs must also form an explicitly admitted source/update pair. An exported job carries its exact ELF; execution checks that ELF against the expected statement instead of substituting the newest build.

The private proving commands accept `--program` for an admitted retained ELF. The source construction commands accept `--programs` for the retained directory. Their native checks, proof verification and development-receipt rejection still apply. Retaining an old program does not admit an unreviewed upgrade or make a historical source root current.

## Source admission

`SourceAdmission` verifies a genuine registry-update receipt under the expected source policy and registry program, then binds the canonical source state and stable checkpoint to that receipt. It checks minimum cumulative work, minimum head height, the exact confirmation depth, matching index/archive commitments and bounded head age/future skew. The trusted clock and admission profile are local configuration; an evidence server cannot choose them. Freshness is required when selecting current source state before payment authorization; it does not expire paid-claim rights or invalidate previously accepted checkpoints used for settlement. The resulting `VerifiedSource` has no public constructor and rechecks freshness when its checkpoint is requested.

A mathematically valid historical receipt remains verifiable as historical evidence. It cannot pass current admission merely because its seal is genuine. The tests use the retained genuine source-update proof and reject it at the current date, as well as rejecting changed policy, work floor, state, checkpoint and development receipts. This does not prove that no heavier valid fork exists; source-chain hashpower, data availability and honest relaying remain assumptions.

```sh
cargo run --locked -p zutti-prover --bin source-prover -- admit \
  --profile /absolute/path/source-admission.bin \
  --proof /absolute/path/source-update.proof \
  --state /absolute/path/state.bin \
  --checkpoint /absolute/path/checkpoint.bin
```

The profile is the canonical `SourceAdmission` encoding created from reviewed deployment parameters. `source-prover update` exports `state.bin` and `checkpoint.bin` alongside the job. This command uses the system clock and has no option to pretend an old block is current. A live deployment profile and current bootstrap have not been established.

## Native transaction validation

The verifier implements pre-Overwinter signature hashing, Overwinter, Sapling and v5/v6 signature hashing through pinned upstream primitives, transparent scripts, value conservation, duplicate-input checks, coinbase maturity/shielding, inclusive expiry, lock time and block signature-operation limits. Coinbase heights use the canonical script prefix; pre-v5 coinbases are reparsed with the actual block branch before authorization checks.

Shielded checks verify Sprout PHGR and Groth16 proofs, JoinSplit signatures, individual Sapling spend/output proofs and authorization/binding signatures, and Orchard/Ironwood proofs with the applicable historical or current circuit key. Coinbase shielded outputs must recover with the consensus zero outgoing viewing key and the required note version. Public verifying-key provenance and reproduction are documented in [the parameter directory](crates/source/parameters/README.md).

Spend state separates all four nullifier sets, checks earlier final anchors and Sprout transaction-local intermediate anchors, and accounts for each shielded pool. Pinned Zebra parameters supply subsidy, founders reward, funding streams and lockbox disbursement schedules. Required outputs are counted distinctly; deferred value and NU6's exact coinbase accounting are checked. Persistent ordered maps make candidate clones share state instead of copying the entire ledger each block.

Tests cover published signature vectors, historical PHGR/Groth16 transactions, modern shielded transactions and altered signatures, public coinbases across upgrade boundaries, the first ten consecutive mainnet blocks, and rejection of wrong founders rewards and excessive coinbase value. These are component and prefix tests, not proof that the entire Zcash history has been replayed or proved.

## Settlement contracts

`SourceRegistry` verifies a journal binding the network genesis, policy, previous head, previous stable checkpoint, new state, new checkpoint and increased cumulative work. It retains accepted checkpoints for later recovery. It has no committee signer or administrative root-insertion function. Its full source program must enforce validity, ancestry, bootstrap and fork/stability rules; the registry's work comparison alone does not establish those facts or identify an unseen heavier fork.

`PrivatePool` binds its domain to the destination chain, pool address, token, source policy and proof profiles. Its five operation modes are deposit, transfer, withdrawal, reservation and settlement. Deposits require exact token receipt. Internal operations consume unique nullifiers and append commitments without transferring ERC-20 principal. Reservations also verify the prepared payment and consume an order-use tag. Terminal settlement consumes one reservation nullifier and appends one output; it has no claim timer. Outputs retain public ciphertexts, and old membership roots remain usable with unspent nullifiers.

Rust and Solidity agree on the 295-byte pool journal, source-update journal and deployment-domain vectors. Contract tests cover claim binding, replay, duplicate commitments, token failures, reentrancy and atomic rollback. State-machine tests use `JournalVerifierMock` to isolate contract behavior. Separate tests verify genuine Groth16 preparation, source-update and pool deposit/transfer/withdrawal evidence. Source and pool proof programs exist, but current-chain Zcash evidence has not been composed with a real funded reservation and its paid/refund settlement. The contracts are not deployed or ready for funding.

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

## Durable source state

The host-only `zutti-source/storage` feature persists authenticated UTXOs, shielded state, transaction-index entries and the permanent header archive in SQLite. Each batch commits its changed leaves, state snapshot and pending witness in one transaction with full synchronous durability. The store rejects stale writers and requires evidence retention before another batch can advance.

Reopening rebuilds the in-memory authenticated trees from stored leaves and checks every root against the saved snapshot. It does not replay previous blocks. Restart cost still grows with the number of retained leaves; this is not a disk-backed Merkle tree or a measured current-chain memory bound. Pending batches are re-evaluated and must match the stored state. Evidence bytes remain untrusted until the proving layer cryptographically verifies their exact program, policy and state binding.

Tests cover both networks, index starts before and after the processed prefix, interrupted transactions, invalid batches, altered leaves, policy changes and concurrent stale writers.

## Resumable source proving

`source-prover` stores validated state separately from proof completion. A pending batch blocks another state advance until an exact genuine Groth16 receipt is cryptographically verified and retained. Opening a store re-verifies its retained predecessor; fake receipts, wrong program pairs, changed policies and altered state fail. Retain both program ELFs and pass `--programs` when continuing with a later binary.

```sh
cargo run --locked -p zutti-prover --bin source-prover -- sync \
  --network testnet --database .data/testnet.sqlite \
  --rpc http://127.0.0.1:18232 --rpc-cookie .data/zebra-rpc/.cookie \
  --jobs .data/testnet-jobs --batch-size 4 --batches 1 \
  --boundless crates/boundless/target/debug/zutti-boundless
```

The RPC returns complete serialized blocks. Requests are bounded, checked against the requested height/hash and passed through native validation; RPC failure never establishes nonpayment. Remote endpoints require HTTPS. Cookie credentials stay out of proof inputs. The runner stops on a competing branch rather than accepting an unverified reorganization.

Execution preflight checks the expected guest journal and measures cycles without generating a local proof. The runner reduces oversized batches before committing them and refuses a single block beyond its configured cycle ceiling. This ceiling is a compute constraint, not a relaxation of validity rules. Successful jobs submit through the existing capped Boundless client. Restarting reuses the pending witness, exact job, saved request and returned result. A request retained before an uncertain broadcast is never replaced automatically; failed/expired requests need explicit reconciliation.

Each invocation processes a bounded number of batches and stops when its limit or the RPC tip is reached. This is not an installed background service. No current-chain bootstrap throughput or proving budget has been demonstrated.

The default cycle ceiling is `268435456`. Use `--cycle-limit` to benchmark larger batches, up to `2147483648` cycles, while retaining the 128-block and 64 MiB witness bounds. Archived source programs retain their original 64-block limit. The block limit is a maximum; transaction-heavy batches may require fewer blocks. Preflight must succeed before the batch is committed or submitted. A retained job above a later invocation's limit stays pending. This option does not raise the configured ETH price ceiling or enable automatic funding.

`init-store`, `advance-store`, `export-store`, `import-store`, `accept-store` and `store-status` expose the individual recovery steps. Status distinguishes the validated height, proved height, pending work and current-chain admission. Only verified evidence advances the proved height.

Historical transaction paths can be reconstructed from stored first-inclusion heights for an archived stable root. `SourceCandidate::prove_at` checks the reconstructed root against the authenticated header archive; later transactions stay absent from that earlier root. Rebuilding these paths currently scans retained index entries and is not an optimized historical-query service.

## Composable source ranges

`source-prover range` exports a validity proof job for a consecutive block range. The claim commits to both endpoint states; it proves a transition conditional on its starting state and does not establish a prefix from genesis. Its witness contains public block data and authenticated state accesses, without requiring the preceding range's proof.

`aggregate-ranges` combines two to four genuine range receipts. Every boundary must match the entire preceding endpoint claim, including policy, height, tip, work, history index and state commitment. Gaps, reordered ranges and substituted states fail. A tree of these jobs can combine independently proved ranges.

`fold-range` combines a genuine full-prefix receipt with a genuine range receipt whose starting claim exactly equals that prefix. The source guest verifies both receipts before emitting the existing full-prefix claim. This preserves the genesis anchor and the source-registry interface. Range receipts use a separate claim encoding and proof kind; they cannot advance the full-prefix store or substitute for source admission.

Genuine Boundless Groth16 fixtures cover Testnet genesis, independent ranges 0→2 and 2→4, their 0→4 aggregate, and the final genesis-anchored prefix. Regression tests verify every receipt, reproduce both native transitions and check that range evidence cannot substitute for a full-prefix proof. The leaf jobs took 63 and 91 seconds from submission to receipt; aggregation took 146 seconds and prefix folding took 152 seconds. These are observations for four early blocks, not current-chain throughput or swap latency.

`source-prover bootstrap` persists a separate range queue. It validates blocks sequentially while allowing independent proof jobs to finish in either order. State changes and queued witnesses commit atomically. The default window retains at most 16 ranges and 128 MiB of pending witnesses; configurable hard bounds are 64 ranges and 512 MiB. Completed range proofs replace their witnesses. Two to four consecutive proved ranges can be compacted into one aggregate; only a verified full-prefix receipt advances the proved base and prunes covered queue entries. Reopening authenticates all state roots and re-verifies retained Groth16 receipts.

```sh
source-prover bootstrap --network testnet --confirmations 100 \
  --database .data/bootstrap.sqlite init --genesis .data/genesis.bin
source-prover bootstrap --network testnet --confirmations 100 \
  --database .data/bootstrap.sqlite fill --rpc http://127.0.0.1:18232 \
  --rpc-cookie .data/zebra-rpc/.cookie --batch-size 4 --batches 2
source-prover bootstrap --network testnet --confirmations 100 \
  --database .data/bootstrap.sqlite export --out .data/genesis-job genesis
source-prover bootstrap --network testnet --confirmations 100 \
  --database .data/bootstrap.sqlite export --out .data/range-1-job range --batch 1
```

`export` executes the exact guest before writing a job with its measured cycle budget. Submit each exported `job.json` using the existing capped Boundless client, preserve its request, and import its returned result with `private-prover import-boundless`. Use `accept --proof <artifact> range --batch 1` for leaf receipts and `accept --proof <artifact> prefix` for the genuine genesis receipt. Once consecutive ranges are proved, `export --out <job> aggregate --batch 1 --batch 2` prepares their aggregation; `accept --proof <artifact> aggregate --batch 1 --batch 2` retains it. `export --out <job> fold --batch 1` joins that range to the retained full prefix, whose receipt is collected with `accept ... prefix`. Exact repeated acceptance is idempotent. `status` distinguishes native height, full-prefix proved height and individual range completion.

With a retained prefix proof, `export --out <job> continue --batch 1` can prove the first unproved range directly as a full-prefix extension. This combines predecessor verification and block validation in one request, avoiding a separate range receipt and fold. It must fit the cycle ceiling including predecessor verification. Import and accept the returned receipt with `accept ... prefix`. An existing paid range request must be completed through its original range/fold path; direct export does not replace or reconcile that request. The automated range runner continues using its existing range/aggregate/fold flow.

`bootstrap run` fills a bounded window, submits independent leaves, collects them with bounded concurrency, builds an aggregation tree and folds the result into the retained full prefix. Submission is sequential to avoid concurrent account-nonce allocation. `--workers` bounds concurrent collectors; `--max-jobs` limits new submissions in that invocation, and the existing Boundless ETH price ceiling and `FundingMode::Never` still apply. A saved request is reused only if its image and journal match the exact queued job. An uncertain request is never replaced automatically. Completed receipts stay on disk if another collector fails, and restarting re-verifies them before acceptance.

```sh
source-prover bootstrap --network testnet --confirmations 100 \
  --database .data/bootstrap.sqlite run \
  --rpc http://127.0.0.1:18232 --rpc-cookie .data/zebra-rpc/.cookie \
  --jobs .data/bootstrap-jobs --batch-size 64 --batches 2 \
  --rounds 1 --workers 2 --max-jobs 5 \
  --boundless crates/boundless/target/debug/zutti-boundless
```

These commands require an initialized queue. Keep exact job/request/result files and matching program ELFs for recovery. A bounded run is not an installed daemon, a demonstrated full-history catch-up or canonical-chain admission. Modern block cost and sustained proving throughput remain measurements to establish.

## Historical workload profiling

`source-replay` validates a length-framed stream of public blocks from the exact genesis and saves native progress in a separate replay database. Selected intervals retain authenticated transition witnesses atomically with that progress. This database has a distinct schema and cannot be opened as a proved prefix or bootstrap queue. It contains no source receipts and never advances source admission.

```sh
cargo build --locked --release -p zutti-source --features storage --bin source-replay
python3 scripts/profile-source-history.py \
  --network testnet --rpc http://127.0.0.1:18232 --rpc-cookie /path/to/.cookie \
  --replay target/release/source-replay --programs /path/to/pinned-programs \
  --database /path/to/native-replay.sqlite --out /path/to/profile \
  --through-height 2000 --sample-start 1000 --sample-size 16
```

The driver checks the node network and retained parent hash, streams bounded blocks with backpressure and resumes from the last committed batch. A failed batch leaves earlier progress intact. The replay database and sample witnesses are profiling artifacts; they do not replace the runtime store or a genesis-anchored proof.

`--index-start` selects the benchmark's payment-lookup boundary and changes its policy. Deferring that index still validates every preceding block and transaction; it does not substitute a trusted state snapshot for replay.

Export a retained sample under the exact source/update program pair, then measure its guest execution:

```sh
source-prover export-range --programs /path/to/pinned-programs \
  --witness /path/to/profile/samples/1000-1016/witness.bin --out /path/to/range-job
source-prover execute --job /path/to/range-job
```

This measures a conditional range transition. Its starting state has native replay evidence, not a proved prefix. The range cannot substitute for a full-prefix receipt. Neither command purchases proofs; any paid range test still uses the separately capped Boundless client.

## Source upgrade support

The source program implements NU7 using `zebra-chain` 14.0.0 and the matching Zcash transaction libraries. Public Testnet activates NU7 at height `4_465_026`, branch `0x77190ad9`. Difficulty validation retains 113 ancestor headers, applies the 102-block averaging window and 25-second spacing at activation, and uses the new Testnet minimum-difficulty threshold. Parsing enforces per-pool and combined shielded action limits. Transaction checks reject v4 at NU7, retain valid v5/v6 formats, enforce Orchard freezing, and bind authorization signatures to the active branch.

Monetary transitions check the complete coinbase against the reference subsidy and recipient rules. Snapshots authenticate the NSM reserve from activation, apply the 60% fee contribution with consensus rounding, and use the parent reserve for reissuance from its deployment height. The NSM seed is a protocol constant, not a committee checkpoint. Pre-NU7 monetary encodings remain unchanged; post-activation snapshots include the reserve explicitly. History commitments reset at NU7 and continue using V3 Ironwood metadata.

Retained V1/V2 source programs cannot admit Testnet heights at or above NU7. Their earlier genuine receipts remain verifiable. Keep the matching binary and both ELFs when recovering older jobs: new native validation rules are not interchangeable with an older program image. Neither NU7 code support nor a synchronized Zebra node establishes a current-chain Zutti proof.

Guest builds remap workspace and Cargo registry paths and preserve the selected `CARGO_HOME` when invoking the RISC Zero compiler. Native builds still produce different images across checkouts. Pin the exact ELFs from the canonical build, and keep its compiler version, SDK, lockfiles and binary with the proof profile. Neither path remapping nor matching source code makes another build's image interchangeable.

Reference: [Zebra NU7 release](https://github.com/ZcashFoundation/zebra/releases/tag/v7.0.0-rc.0).

## Next verification milestone

Establish current-chain bootstrap evidence, measure realistic block proving cost and throughput, and complete fork/stability admission. Genuine historical prefix receipts do not establish that a submitted branch is the heaviest available chain. Cross-chain reservation/settlement composition, live Ethereum finality and independent recovery remain required. Boundless supplies proving capacity for implemented programs.

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

Add `--profile <output.pb>` to `source-prover execute` to record a pprof cycle profile. Profiling executes the retained program and input, checks the exact expected journal, and generates no proof or market request. Compare the same input and segment limit when measuring an optimization; a cycle reduction alone does not establish a market price.

Use `--candidate <program.elf> --profile <output.pb>` to execute a candidate program against the retained input and expected journal. The report includes both program images and whether they match. This diagnostic comparison does not admit the candidate program, generate a proof, advance stored source state or change the deployment policy.

[The source benchmark](benchmarks/source-proving-2026-10-18.json) records immutable input hashes, program identities and execution measurements. On the same 64-block input, shared witness storage, contiguous accelerated hashing and streamed authenticated paths reduced user cycles by 14.81% and segments by 17.97%. A separate 128-block testnet range executed in 1,885,148,375 user cycles across 2,027 segments. These early blocks do not establish throughput or cost for later transaction-heavy history. Diagnostic comparisons against an old statement are listed separately from evidence proved under the optimized program's own policy.

[The direct-extension benchmark](benchmarks/source-proving-2026-10-19.json) retains a genuine proof that verifies the genesis predecessor and validates the same 128 blocks in one request. Its journal exactly matches the earlier range/fold result. Native replay accepts this receipt without an intermediate range proof and reopens the completed queue with the same authenticated state. Submission to fulfillment took 325 seconds, and the recorded proof fee was 41,617,228,341,561 wei, excluding gas and infrastructure. This was 35.90% less than the earlier range and fold fees combined. Market conditions differed; this observation does not establish future prices, modern-block throughput or the cost of a complete bootstrap.

Boundless fulfilled that 128-block range in 30 minutes 21 seconds at a proving fee of 39,952,539,207,899 wei. Combining it with the genuine optimized-genesis receipt took 2 minutes 23 seconds and cost 24,970,337,004,936 wei. All three receipts passed independent native Groth16 verification. Including genesis, the three proving fees total 91,557,902,351,434 wei, excluding gas, input uploads and local execution. This is a historical genesis-to-128 proof under the optimized program; current-chain bootstrap remains unfinished.

A direct extension through Testnet blocks 1–112 executed in 1,838,191,924 user cycles across 1,967 segments, including in-guest verification of the genuine optimized-genesis Groth16 receipt. Its exact expected journal matched. This execution produces no proof; market time and fees for direct 112-block extensions remain unmeasured. The direct path needs one request per extension instead of a range request followed by a fold request.

The direct 128-block extension also fits the existing ceiling: 2,068,555,597 user cycles and 2,214 segments. It verifies the genuine genesis predecessor and produces exactly the same expected full-prefix journal as the 128-block range/fold path. This direct-input measurement is execution only; it is not a separate paid proving benchmark.

The existing `--index-start` option can place the payment lookup boundary at deployment while full consensus validation still starts at genesis. A separate range over blocks 1–128 with index start 128 used 1,765,386,732 user cycles and 1,888 segments, 6.35% fewer user cycles than the zero-start 128-block range. Orders require an anchor strictly after that boundary, and source registry admission requires a stable height at or after it. This changes the source policy and supported payment history; it is not a trusted chain checkpoint. The measurement is execution only, and no deployment boundary has been selected.

The first mainnet-genesis job executed in 24,111,161 cycles after replacing repeated empty-tree construction with 66 consensus constants independently checked against the pinned upstream implementations. The preceding implementation used 1,056,212,137 cycles. Boundless fulfilled the genuine genesis proof in 76 seconds after submission; it cost 35,790,816,373,743 wei from the proving deposit. Local cryptographic verification passed. The retained public fixture tests verification and altered-seal rejection.

A continuation through mainnet blocks 1 to 4, including in-guest verification of the genuine genesis proof, executed in 253,879,014 cycles. These measurements cover this early public prefix. They do not establish current-chain proving cost, a current bootstrap or a completed swap.

The source registry enforces an immutable, nonzero `minimumWork` as well as increasing head work. A deployment must choose a floor appropriate to its source network before accepting liquidity. The local script uses `1` only behind a chain-31337 guard. This work floor does not replace full chain validation or assert a trusted state root. The registry compares proved histories; learning the heaviest available history still depends on permissionless relayers, data availability and the source chain's hash-power assumptions. A reorganization past the accepted stable prefix halts incompatible updates.

`source-update-proof.json` is genuine public registry-update evidence for the early mainnet prefix. The default Solidity test verifies it with the upstream Groth16 verifier and checks changed state, altered seal, replay and bootstrap-floor rejection. This fixture is not a current-chain bootstrap.

## Private pool proof execution

The separate pool guest checks canonical private notes, owner signatures, Merkle membership, secret-derived nullifiers, value conservation and output encryption. Its uniform journal also supports conditional reservations and settlement predicates. The contract separately requires the prepared-payment proof and an accepted source checkpoint for cross-chain operations. A note proof alone cannot authorize ZEC payment.

Genuine public demonstration receipts cover deposit, transfer and withdrawal. The default Rust and Solidity tests verify those receipts, altered-seal rejection, proof-kind binding and replay protection. `PoolLifecycleTest` deploys the actual Groth16 verifier and executes the three operations with a local demonstration token; it does not use a proof mock. The deterministic note keys are intentionally public demonstration data.

The same cycle was broadcast to a fresh Anvil chain. It deposited 200 local demo USDC, transferred private notes, withdrew 100, retained 100 in token backing and rejected every replay. `scripts/execute-local-pool.py` reproduces execution from retained job data and refuses non-loopback RPCs or chain IDs other than 31337. `contracts/script/LocalPool.s.sol` and the public note generator have the same local-chain restriction. They do not deploy or fund a live-network pool.

| Operation | Measured guest cycles |
| --- | ---: |
| Deposit | 4,696,038 |
| Private transfer | 15,690,710 |
| Withdrawal | 11,014,191 |

`private-prover prove-pool` accepts the encrypted archive/key-file transport used by the other private component provers. `execute-pool` measures a locally supplied owner-only witness, and `verify-pool` verifies an existing artifact. The public generator only creates deterministic local-chain demo witnesses. Real private swap witnesses remain excluded from the public Boundless transport.

These checks establish the private-note execution path. They do not demonstrate a mined Ironwood swap payment, destination finality, full current-chain source bootstrap or independent swap recovery.
