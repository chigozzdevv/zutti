# Zutti

Cross-chain RFQ swaps between native shielded ZEC on Zcash Testnet and private USDC notes backed by Sepolia USDC.

The selected source model is genesis-bound SPV: Zutti verifies header rules and complete payment-window coverage, while relying on miners to enforce transaction validity. There is no checkpoint committee or discretionary refund authority. Full-history transaction-validity proving is paused.

## Status

Protocol rules, wallet authorization gates, SPV programs, private-pool contracts, recovery tooling and RFQ packages are implemented. The current SPV program has a genuine genesis receipt. Conditional swap programs have execution coverage but do not yet have the complete genuine proof chain, a live Sepolia deployment or an operational SDK `accept()` flow.

The [remaining work](#remaining-work) includes the unresolved [refund security review](zutti.md#94-refund-security-review). Component tests and downloaded chain data are not evidence of a completed swap.

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

The adapter exports public SPV jobs, checks their program/input/journal hashes and imports cryptographically verified Groth16 receipts. Recursive jobs use separately built guests that verify predecessor seals inside the zkVM; the pinned [Boundless input format](https://docs.rs/boundless-market/2.0.1/boundless_market/input/struct.GuestEnv.html) cannot carry native host assumptions. Two genuine Boundless source receipts now compose through Testnet block 4, and both pass read-only verification against the deployed Sepolia RISC Zero router. This establishes early header-prefix composition, not current-chain admission or live settlement. Destination settlement remains on Sepolia regardless of the market used to buy compute.

Paid jobs, rentals and parameter downloads require an explicit budget. Routine setup does not launch them. Faucet-funded marketplace capacity has not been established.

## Remaining work

1. Measure useful header batch sizes and sustained proving throughput. The two linked early-history receipts establish composition; they do not establish the complete bootstrap cost or timing.
2. Generate genuine binding, pool deposit/reservation, complete payment/absence, terminal and withdrawal proofs under exact release identities, with Ethereum-compatible wrapping where required.
3. Prove headers from genesis through the demo's current Testnet height and maintain fresh source admission.
4. Resolve the [refund lower-bound review](zutti.md#94-refund-security-review) before enabling funded refunds. Guest execution and mutation tests do not establish that cryptographic argument.
5. Deploy the actual Sepolia verifier/registry/pool profile and verify finalized reservation evidence before signing Zcash payments.
6. Connect scanning, spendable-note selection, native Ironwood proving/signing/broadcast, USDC balances and operational SDK `intent.accept()` fulfillment. Generate dedicated Testnet wallets and provide their public funding addresses at this stage.
7. Demonstrate real paid, unpaid, expiry-boundary, delayed-claim, reverse-trade and independent-recovery cases, then deliver the application flow and submission materials.

Completion needs genuine proof receipts, real chain transactions, actual token backing and usable recovery. A proof submission, synchronized node or successful component test is insufficient. Permanent source-halt recovery remains unresolved; conditional funds can remain locked when accepted source evidence becomes unavailable or invalidated. No administrator or timer may turn that failure into a refund for an already-paid trade.

## SPV workflow

The host is an isolated Cargo workspace. Set `CARGO_HOME` to the populated Cargo cache and `SPV` to the built `zutti-spv-prototype` executable. Each output directory must be new. Use reviewed source-policy values; the tiny regression policy is not a deployment profile.

```sh
"$SPV" build --kind source --guest-target "$GUEST_TARGET" --out "$SOURCE_PROGRAM_DIR"

python3 scripts/spv_headers.py \
  --endpoint "$SOURCE_RPC" --cookie "$SOURCE_COOKIE" \
  --start-height 0 --count 1 --out "$EXPORTS/genesis"

"$SPV" bootstrap init --state "$STATE" --program "$SOURCE_PROGRAM_DIR/program.elf" \
  --genesis "$EXPORTS/genesis/headers.bin" --confirmations "$CONFIRMATIONS" \
  --window-blocks "$WINDOW_BLOCKS" --valid-through "$VALID_THROUGH"

python3 scripts/spv_source.py --binary "$SPV" --state "$STATE" --work "$WORK" \
  --endpoint "$SOURCE_RPC" --cookie "$SOURCE_COOKIE" --batch-size 16

"$SPV" bootstrap status --state "$STATE"
```

The exporter uses loopback RPC and an owner-only cookie beside an existing node. Supply `--container` when using the node's Docker network namespace. It exports consecutive 1,487-byte headers in atomic bounded batches and checks RPC hash/link consistency. Export does not authenticate consensus or generate proofs. The Rust preparation step independently verifies its own header rules and predecessor.

The source worker prepares by default; `--prove` explicitly enables supervised local proving. `--follow` continues within job/time limits. `--proof-seconds`, `--rss-mib`, `--cycle-limit` and `--segment-po2` are separate guards. The supervisor is `crates/prover/spv/run.py`; it limits child threads and blocks HTTP downloads through child proxy settings. These guards constrain a process, not a provider's billing. Smaller segments can reduce peak memory while increasing overhead. Public headers may use composite receipts; other proof dependencies retain their required succinct/Groth16 formats even when Testnet witnesses are public.

The store keeps exact programs, policies, witness/journal manifests and genuine receipts. Status derives progress from verified receipts, not a mutable cursor or server label. Restart rechecks accepted evidence and rejects changed programs, journals, seals, missing jobs and incompatible predecessors. One pending batch is retained on failure; resizing/cancellation is not implemented. Preserve the complete state directory.

Import an externally generated genuine header receipt with its exact pending batch:

```sh
"$SPV" bootstrap accept --state "$STATE" --batch "$BATCH" --receipt "$RECEIPT"
```

An empty registry initializes from a genuine genesis-proved stable prefix plus the required confirmations. It does not accept a server-picked recent checkpoint. Admission preparation and execution are separate from proof generation and Ethereum wrapping:

```sh
"$SPV" prepare-admission --header-program "$SOURCE_PROGRAM_DIR/program.elf" \
  --previous "$STABLE_RECEIPT" --confirmations "$CONFIRMATION_HEADERS" --out "$ADMISSION_INPUT"

"$SPV" run-admission --program "$ADMISSION_PROGRAM" \
  --witness "$ADMISSION_INPUT/witness.bin" --previous "$STABLE_RECEIPT" --out "$ADMISSION_OUTPUT"
```

`run-admission` executes by default; `--prove` requests proof generation. Later advances also supply `--stable-headers` to preparation and name the genuine prior prefix. Freshness, work, competing-history observation and real contract verification still apply.

The `payment-binding`, `coverage` and `settlement` Cargo examples exercise the corresponding relations. Use `--help` for their exact proof inputs. Execution with unresolved assumptions is not settlement evidence. Signing keys are not part of the public proving transport.

### Boundless jobs

Build source, admission and pool programs with `build --boundless`. This pins a different image and source policy from the native-assumption profile. Existing receipts do not authenticate the replacement program. Binding is stateless and uses its existing build. Use the adapter commands to execute these inputs; the native `run`, `pool` and `run-admission` commands use host assumptions instead.

```sh
"$SPV" build --kind source --boundless --guest-target "$GUEST_TARGET" --out "$SOURCE_PROGRAM_DIR"

"$SPV" boundless export --kind source --public \
  --program "$SOURCE_PROGRAM_DIR/program.elf" --witness "$WITNESS" \
  --dependency "$PREVIOUS_RECEIPT" --cycles "$CYCLE_LIMIT" --out "$JOB_DIR"

"$SPV" boundless execute --job "$JOB_DIR/job.json"

"$BOUNDLESS" --network "$MARKET_NETWORK" --config "$BOUNDLESS_CONFIG" \
  submit --job "$JOB_DIR/job.json" --request "$REQUEST"

"$BOUNDLESS" --network "$MARKET_NETWORK" --config "$BOUNDLESS_CONFIG" \
  collect --request "$REQUEST" --out "$FULFILLMENT"

"$SPV" boundless import --job "$JOB_DIR/job.json" --proof "$FULFILLMENT" --out "$RECEIPT_DIR"
"$SPV" bootstrap accept --state "$STATE" --batch "$BATCH" --receipt "$RECEIPT_DIR/receipt.bin"
```

`--public` explicitly authorizes publishing the witness. Omit `--dependency` for genesis, binding and stateless pool operations. Supply one exact predecessor receipt for source/admission and two in the required order for reservation/settlement. Every supplied dependency must already have a genuine Groth16 seal under the pinned parameters; composite or succinct receipts must be wrapped first. Native-assumption guests cannot accept dependencies through this transport.

Export and execution do not purchase proofs. `submit` can spend the configured market budget; `collect` retains fulfillment data, while `import` cryptographically verifies it before publishing `receipt.bin`, `seal.bin` and verification metadata. Program, stdin and journal changes invalidate a version-2 job. Import and export refuse an existing output directory. Source acceptance rechecks the exact pending batch and does not imply admission to the on-chain registry.

The retained execution measurements show about 48% fewer pool-operation cycles after removing duplicate validation. Separately, the receipt-verification probe consumed 183,494,181 cycles for one genuine historical Groth16 receipt, with zero host assumptions. Each inline dependency adds verification work; these figures are not complete proof-time or market-price estimates. `build --kind receipt-check` builds that regression probe, not a settlement program. No new SPV composed proof was generated by these execution checks.

## Ethereum evidence

```sh
cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  finality --config CONFIG.json --evidence FINALITY.json

cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  reservation --config CONFIG.json --request REQUEST.json --evidence RESERVATION.json

cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  inventory --config CONFIG.json --request INVENTORY_REQUEST.json --evidence INVENTORY.json
```

The native verifier checks pinned Ethereum consensus/bootstrap configuration, finalized execution-header binding and EIP-1186 account/storage proofs. Inventory proofs authenticate pool and source-registry state under the same finalized root. Locally accepted terms, code hashes and proof identities come from the pinned deployment profile, never replacement values chosen by the evidence server. Successful CLI JSON is a local verification result, not transferable proof authority.

Use the full `SpvFundingProfile::authorize` entry point for funder authorization. Local consent checks alone do not authenticate chain state. The ZEC-payer adapter separately verifies the finalized reservation before releasing spending authorization. [zutti.md](zutti.md#115-implemented-spv-composition-and-ethereum-authorization) specifies the relation and bootstrap assumptions.

## Evidence and specification

[Retained evidence](crates/prover/spv/evidence.json) records exact program identities, the genuine genesis proof/audit, two linked Boundless source receipts with actual fees and fulfillment times, historical execution samples and failed bounded proof attempts. Public proof regression data remains in the component fixtures. Its scope and upstream provenance are documented in [zutti.md](zutti.md#24-verification-parameters-and-fixture-provenance).

[zutti.md](zutti.md) contains the architecture, SDK contract, source/payment/refund rules, trust and privacy boundaries, security-review questions and acceptance criteria. These two root files are the project's documentation. Earlier experiments and removed narrative files remain recoverable from Git history.
