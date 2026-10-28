# Zutti

Encrypted RFQ swaps from native shielded ZEC to private USDC notes on Ethereum.

The Testnet prototype uses a deployment-pinned Zcash checkpoint and SPV proofs. It relies on that selected checkpoint and on miners enforcing transaction validity. The active profile supports ZEC → USDC; reverse swaps and nonpayment refunds are disabled.

## Status

**One funded Testnet swap and withdrawal are complete:** 0.01 shielded Testnet ZEC → 1 private Sepolia USDC → 1 public USDC at the taker's withdrawal address. The source payment, funded reservation, private payout and withdrawal used genuine proofs and real chain transactions.

| Step | Observed result | Evidence |
| --- | --- | --- |
| Reservation | 1 USDC reserved and verified against Ethereum finality | [Sepolia transaction](https://sepolia.etherscan.io/tx/0x7dba135cc675ecb58d18b7499fc78bc0aa911b21b7e7e0bc04153fa42ab73dc3) |
| Zcash payment | 0.01 shielded ZEC received; mined at 4547730 with twenty following blocks | `eca18c82bf3bb690f23a11f98ea0cbf6d664e8b87f400385fe0780ac8c38c11b` |
| Private payout | 1-USDC note decrypted; ownership and unspent state verified against Ethereum finality | [Sepolia transaction](https://sepolia.etherscan.io/tx/0x5a7f3ba4957a6c227030b1379e58c8ddb463df168f43156deef441b70007eca7) |
| Withdrawal | 1 USDC transferred to the taker; transaction and receipt roots authenticated against Ethereum finality | [Sepolia transaction](https://sepolia.etherscan.io/tx/0x4ac40981f66bb1044b0f6bd409c4aa5e68b84e5beced22f1e7d7a9180afb5592) |

The maker wallet recovered the 0.01-ZEC note. The taker recovered 0.9899 ZEC of change after a 0.0001-ZEC network fee. Its public USDC balance increased from zero to 1, and the active pool's token balance returned to zero. The quoted 100-USDC-per-ZEC rate was a chosen demo quote.

[The evidence record](deploy/ethereum/swap-proof.json) retains proof identities, journals, seals, actual transactions, fee records, owner-recovery checks and a deployed-pool rejection of a forged withdrawal. Fourteen proof requests, including setup and earlier attempts, cost **0.00047583 ETH**, within the approved 0.000625-ETH cap. These were real proving fees; ZEC, USDC and settlement gas used Testnet assets.

[The deployment record](deploy/ethereum/sepolia.json) pins the actual contracts, token and programs. The [source profile](deploy/source/testnet/profile.json) retains the shared header/admission receipts through height 4,547,456; the payment-specific admission in the swap evidence advances the accepted stable prefix to 4,547,740, with head 4,547,750. Historical snapshots require fresh evidence before new reservations. The earlier deployment's separate 10-USDC deposit and recovery records remain under [legacy records](deploy/legacy/20261027-r2/ethereum/sepolia.json).

## Flow

1. The wallet requests encrypted, authenticated maker quotes and accepts fixed terms.
2. The maker spends prefunded private USDC into the agreed conditional reservation.
3. The payer verifies the finalized reservation, accepted source state and remaining payment window before signing ZEC.
4. A native Ironwood payment is broadcast. Its binding and confirmed inclusion are proved under the pinned source policy.
5. The pool verifies the proofs and creates the precommitted private USDC note.
6. The owner verifies that note and explicitly authorizes a public ERC-20 withdrawal.

The private receiving key owns a note backed by tokens in the pool. It is not an ERC-20 address. The recorded flow uses the headless SDK RFQ, persistent native owner coordinator and SDK withdrawal adapter. [Quote execution](scripts/quote_testnet.mjs) and [withdrawal execution](scripts/withdraw_testnet.mjs) show the adapters. A React client now provides quote selection, review, progress, activity and withdrawal screens through a session-scoped SDK gateway. The native owner-wallet connector still needs packaging; no browser extension integration is claimed. [zutti.md](zutti.md) separates the broader intended SDK from implemented capabilities.

## Boundaries

- The source model is checkpoint-bootstrapped SPV. It does not independently prove all Zcash transaction validity or genesis-to-tip history. PoW reorganizations, unavailable data and source conflicts remain relevant.
- Testnet proving inputs are public through Boundless/IPFS, including note openings and test private-USDC nullifier secrets. Signing and viewing keys stay local. This demo does not establish production witness confidentiality.
- Nonpayment, missing evidence or a source halt cannot trigger an administrator or timer refund. Reservations can remain locked. Reverse swaps and nonpayment refunds need separate protocol work and review.

The owner's restored encrypted record reproduced the payment-window witness and supplied the actual payout witness without the maker's private funding data. That establishes the recorded paid-claim recovery path; it is not a nonpayment-refund demonstration.

## Layout

```text
crates/protocol/        encodings, commitments, fixed terms and note rules
crates/wallet/          native Zcash payments, private notes and recovery
crates/wallet/ethereum/ finality, storage evidence and authorization
crates/source/          Zcash parsing, headers and payment coverage
crates/prover/          proof programs, verification and artifacts
crates/prover/spv/      active SPV host and proof regression tests
crates/boundless/       proof-market submission and collection
crates/recovery/        standalone recovery CLI
contracts/             source registry and USDC-backed private pool
packages/sdk/           application SDK
packages/maker-sdk/     maker quoting SDK
packages/runtime/      encrypted RFQ and persistence
packages/schema/       shared validation
packages/bindings/     native and contract interfaces
server/src/features/   RFQ relay
server/src/shared/     HTTP utilities
scripts/               build, verification and owner workers
client/src/app/        responsive application shell and styles
client/src/components/ swap, progress, activity and dialogs
client/src/shared/     application transport and formatting
deploy/                retained deployments and genuine evidence
```

## Development and verification

Run the application locally:

```sh
pnpm install --frozen-lockfile
pnpm dev
```

Open `http://127.0.0.1:5173`. The app API listens on loopback port 3002. For the production build, run `pnpm build:client` followed by `pnpm start:client`, then open `http://127.0.0.1:3002`. Fonts are bundled locally.

Without a wallet adapter, the interface shows the completed Testnet evidence and a disconnected wallet. It does not fabricate quotes or balances. A reviewed local module can be supplied with `ZUTTI_WALLET_MODULE=/absolute/path/wallet.mjs`; it must export `connect_wallet()`, returning `{ wallet, label }`, where `wallet` implements the existing SDK `WalletProvider`. The gateway pins the deployment, uses the SDK's encrypted RFQs and execution callbacks, and keeps signing material out of browser responses. Origin checks, HTTP-only owner sessions and idempotent acceptance/withdrawal protect the local interface. The gateway retains operations across page reloads while its process runs; recovery across gateway restarts requires the durable native owner adapter. Sessions are local to this one owner process, not a multiuser hosted wallet service.

App startup and read-only browsing do not request proofs or send transactions. New execution still requires the wallet's fresh evidence and spending authorization. `pnpm build:client`, `pnpm lint:ts`, and the TypeScript tests cover the client and gateway; the gateway tests do not establish a new funded swap.

Use the pinned Rust, RISC Zero 3.0.5, Foundry, Node.js and pnpm versions. Existing caches can be reused through `CARGO_HOME` and `CARGO_TARGET_DIR`.

```sh
pnpm build
pnpm typecheck
pnpm lint:ts
pnpm test:ts
cargo test --locked --manifest-path crates/prover/spv/Cargo.toml --features recovery --test ethereum
cargo test --locked --manifest-path crates/prover/spv/Cargo.toml --features wallet-proving,recovery --test wallet_inventory
cargo test --locked --manifest-path crates/wallet/ethereum/Cargo.toml --features spv-fulfillment
forge test --root contracts
```

These tests verify retained genuine receipts and replay finalized Ethereum evidence at its recorded verification time. Live funding, signing and withdrawals enforce current freshness. Tests do not purchase proofs. `pnpm test:proofs` generates expensive local proofs; `pnpm test:boundless` submits a paid request.

RISC Zero supplies the proof programs and Boundless supplies remote compute. The owner coordinator retains requests and transaction identities across interruptions; confirmed operations are collected without resubmission. The test-run ledger is scoped to the completed trade and closes after authenticated withdrawal. No new spending is implicit in setup or verification.

## Execution improvements

The owner runtime now checks source admission and finalized spendable inventory before quoting, reuses fresh inventory evidence, and continues a retained operation after caller interruption. Payout proving overlaps source admission; source admission still gates the payout transaction. Exact-job execution checks and per-step timings are persisted.

The optimized admission guest reuses the genuine payment-window receipt, reducing its work from **1.912 billion to 242.9 million cycles**. Its isolated [proof](deploy/source/optimized-admission/proof.json) fulfilled in 85 seconds. The [optimized registry and pool](deploy/ethereum/optimized/sepolia.json) now have a completed live SDK run: **0.01 Testnet ZEC → 1 private USDC → 1 USDC withdrawn**. All nine Boundless receipts verified, and the final ERC-20 transfer was independently authenticated against Ethereum finality. Proof fees, including source admission and the maker deposit, totaled **0.0001498 ETH**, within the approved 0.00035-ETH cap. [Evidence and timings](deploy/ethereum/optimized/swap-proof.json).

| From accepting the offer | Earlier run | Optimized run |
| --- | --- | --- |
| Verified private USDC ownership | 3 h 19 min | 2 h 42 min |
| Finalized public withdrawal | 3 h 38 min | 3 h 14 min |

The observed reductions were about 37 minutes to private ownership and 24 minutes through withdrawal. These are individual runs with different source ranges and marketplace conditions; deployment and wallet/source warm-up are excluded. The payment-evidence request still took **66 min 48 s**, so practical swap latency remains unresolved. Zcash confirmation depth and Ethereum finality are unchanged.

## Remaining work

1. Reduce payment-evidence latency and measure repeated complete swaps.
2. Package the native owner-wallet connector and verify the browser flow with a newly authorized funded swap.
3. Build the maker dashboard and replace fixed-zero reserved/active-swap summaries with authoritative inventory and obligation tracking.
4. Operate and measure a continuous source feed and multiuser maker/prover service.
5. Establish confidential proving transport and complete independent security review. Future nonpayment refunds remain gated by the [refund-window review](zutti.md#94-refund-security-review).
