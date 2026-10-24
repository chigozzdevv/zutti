# Zutti

Cross-chain RFQ swaps between native shielded ZEC on Zcash Testnet and private USDC notes backed by Sepolia USDC.

The selected source model is genesis-bound SPV: Zutti verifies header rules and complete payment-window coverage, while relying on miners to enforce transaction validity. There is no checkpoint committee or discretionary refund authority. Full-history transaction-validity proving is paused.

## Status

Protocol rules, wallet authorization gates, SPV programs, private-pool contracts, recovery tooling and RFQ packages are implemented. The current SPV program has a genuine genesis receipt. Conditional swap programs have execution coverage but do not yet have the complete genuine proof chain, a live Sepolia deployment or an operational SDK `accept()` flow.

The remaining work is the [Testnet delivery plan](docs/testnet-delivery.md). The [refund-window review](docs/refund-window-review.md) remains a security gate. Component tests and downloaded chain data are not evidence of a completed swap.

## Layout

```text
crates/protocol/        encodings, commitments, fixed swap terms and note rules
crates/wallet/          payment preparation, private notes and recovery records
crates/wallet/ethereum/ Ethereum finality, storage evidence and authorization
crates/source/          Zcash parsing, header verification and payment coverage
crates/prover/          proof verification, artifacts and operation tooling
crates/prover/methods/  isolated RISC Zero guest programs
crates/prover/spv/      active SPV host, bootstrap CLI and proof regression tests
crates/boundless/       proof-market submission and collection
crates/recovery/        standalone recovery CLI
contracts/             source registry and USDC-backed private pool
packages/sdk/          application SDK
packages/maker-sdk/    maker quoting SDK
packages/runtime/      RFQ transport, encryption, persistence and orchestration
packages/schema/       shared message validation
packages/bindings/     native and contract interfaces
server/src/features/   RFQ relay
server/src/shared/     HTTP utilities
scripts/               build, verification and source workers
deploy/source/         Testnet node configuration
docs/                  source, settlement, refund review and delivery instructions
```

The application client and live wallet/settlement adapters remain unfinished. [zutti.md](zutti.md) specifies the intended product and SDK contract; it does not claim that every proposed method is operational.

## Development

Use the pinned Rust toolchain, RISC Zero 3.0.5, Foundry, Node.js and pnpm versions declared in the repository. `pnpm setup` fetches dependencies; existing caches can be reused with `CARGO_HOME` and `CARGO_TARGET_DIR`.

```sh
pnpm build
pnpm lint
pnpm typecheck
pnpm test
```

For the active SPV path without rebuilding the legacy guest workspace:

```sh
cargo build --locked --manifest-path crates/prover/spv/Cargo.toml
cargo test --locked --manifest-path crates/prover/spv/Cargo.toml --features recovery --tests
cargo clippy --locked --manifest-path crates/prover/spv/Cargo.toml --all-targets --features recovery -- -D warnings
cargo test --locked --manifest-path crates/wallet/ethereum/Cargo.toml --features spv-fulfillment
python3 -m unittest discover -s scripts -p 'test_spv_*.py'
python3 -m unittest discover -s crates/prover/spv -p 'test_*.py'
forge test --root contracts
```

Set `ZUTTI_SPV_BINARY` to the built `zutti-spv-prototype` executable to include the source-worker CLI integration test. Routine tests verify retained receipts and do not purchase proofs. `pnpm test:proofs` generates expensive local proofs; `pnpm test:boundless` submits a paid request.

## Proving and settlement

RISC Zero supplies the proof programs; Boundless supplies remote proving compute. Public source history is shared across swaps. Each swap additionally needs payment binding, reservation, payment-or-absence coverage and terminal settlement evidence.

The current Boundless transport publishes program inputs through IPFS. The Testnet demo explicitly permits public swap witnesses. Those witnesses are not confidential from the prover or others who obtain the published inputs. Wallet signing keys remain local. This demo profile does not establish production witness privacy.

The existing market client has successfully returned genuine proofs for earlier programs. Wiring the new SPV programs and predecessor-proof dependencies into that transport remains unfinished. A marketplace submission or expired request is not a verified proof. Destination settlement remains on Sepolia regardless of the market used to buy compute.

No new paid jobs, rentals or parameter downloads are part of routine setup or this cleanup. Spending requires an explicit budget. Faucet-funded marketplace capacity has not been established.

## Documentation

- [SPV host](crates/prover/spv/README.md): build, guest execution and proof formats.
- [Source bootstrap](docs/spv-bootstrap.md): export existing node headers and resume verified work.
- [Source verification](docs/source-verification.md): header, mining-validity and chain-observation assumptions.
- [Payment binding](docs/payment-window-binding.md): exact payment and bounded refund interval.
- [Settlement and recovery](docs/spv-settlement.md): backed reservation and fixed payout/refund notes.
- [Ethereum verification](crates/wallet/ethereum/README.md): finalized account/storage evidence.
- [Retained evidence](crates/prover/spv/evidence.json): exact program identities, genuine genesis evidence and explicitly unproved execution samples.

Genuine proof fixtures used by regression tests remain in `tests/fixtures` and the SPV host's `fixtures` directory. Older fixtures certify only their original programs. Historical experiments and removed measurements remain recoverable from Git history.
