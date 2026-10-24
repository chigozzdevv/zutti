# Ethereum verification

A native verifier for the Ethereum evidence required before releasing a Zcash spending authorization. This workspace pins Helios consensus verification and isolates Alloy's dependencies from the Zcash proving workspaces.

With `spv-fulfillment`, it also checks inventory and source admission before authorizing a private USDC reservation. `SpvFundingPlan` first checks the locally accepted terms, funder-owned refund and change, input ownership/membership and exact value conservation. `SpvFundingProfile::authorize` then requires genuine binding and anchor receipts plus finalized Ethereum inventory evidence before creating spend authorizations. A native provider must use this complete entry point; the lower-level plan's local consent check does not authenticate chain state.

The verifier authenticates the bootstrap against the configured Ethereum checkpoint, verifies BLS signatures and committee transitions, and uses only the resulting finalized header. It does not force optimistic updates into finality. It checks the complete execution header against that finalized beacon header, then verifies the pool account and three storage values against the execution state root:

- The expected escrow commitment exists.
- The expected order-use tag is recorded.
- The escrow nullifier is absent.

The account proof must match the expected pool address and deployed bytecode hash, including immutable configuration. The wallet must derive the escrow commitment, order-use tag and nullifier from its own reviewed swap. Caller-supplied labels do not prove those bindings. The storage layout is checked against the Solidity compiler by `scripts/check-ethereum-layout.py`.

`ConsensusConfig` and `ReservationRequest` are trusted local inputs. Evidence is untrusted. Configuration must bind the intended network's genesis, fork schedule and a recent independently established finalized checkpoint. The evidence server cannot choose that checkpoint or the expected contract. Ethereum checkpoint bootstrap has the standard weak subjectivity assumption; this does not create a separate Zutti committee. Clock correctness, Ethereum's light-client consensus assumptions and RPC availability still matter. The 14-day configuration cap is not a computation of the network's current weak subjectivity period.

```sh
cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  finality --config CONFIG.json --evidence FINALITY.json

cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  reservation --config CONFIG.json --request REQUEST.json --evidence RESERVATION.json

cargo run --locked --manifest-path crates/wallet/ethereum/Cargo.toml -- \
  inventory --config CONFIG.json --request INVENTORY_REQUEST.json --evidence INVENTORY.json
```

Finality evidence contains `bootstrap`, `updates`, `finality` and the full execution `Header`. Reservation evidence contains `consensus` with that finality evidence and `account` with the EIP-1186 response. Requests contain `chain_id`, `pool`, `pool_code_hash`, `escrow`, `order_tag` and `nullifier`. Unknown top-level fields, oversized files, incorrect proofs and stale or future evidence are rejected. Successful JSON output is a local verification result, not a transferable proof that another process should trust from a server.

Inventory evidence contains `consensus`, `pool` and `source`. The two account/storage proofs must authenticate against the same finalized execution state root and their locally pinned code hashes. They establish a recognized pool root, unspent input nullifiers, an unused order tag, acceptance of the exact source checkpoint, and a non-halted source with enough remaining blocks and an acceptable timestamp. Requests come from the unsigned funding plan and pinned profile, including the registry address/code hash. The provider must not accept replacement expectations from the evidence server. The layout check covers both pools and the packed registry fields used here.

The inventory tests use actual generated Merkle-Patricia proofs, including valid proofs of ineligible states. They do not replace Ethereum consensus signatures or the Zutti proof verifier. Successful live funding authorization remains untested because genuine private binding evidence, current source admission, deployed contracts and a live wallet are still required. The separate bounded-refund security review remains a funding gate.

The tests retain official Ethereum consensus vectors with provenance. They cover finality, committee rotation, changed checkpoints/signatures, future/stale evidence, header binding and generated Merkle-Patricia inclusion/non-membership proofs. They do not establish a live Ethereum reservation, authorize a real Zcash payment or complete a swap. No live bootstrap or pool deployment is configured here.

Primary references: [Ethereum weak subjectivity](https://ethereum.org/developers/docs/consensus-mechanisms/pos/weak-subjectivity/), [Helios pinned source](https://github.com/a16z/helios/tree/2af9b41887e069144ee56cd710f3ddca195473f8/ethereum/consensus-core), [EIP-1186](https://eips.ethereum.org/EIPS/eip-1186).
