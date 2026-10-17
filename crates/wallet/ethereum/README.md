# Ethereum verification

A native verifier for the Ethereum evidence required before releasing a Zcash spending authorization. This workspace pins Helios consensus verification and isolates Alloy's dependencies from the Zcash proving workspaces.

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
```

Finality evidence contains `bootstrap`, `updates`, `finality` and the full execution `Header`. Reservation evidence contains `consensus` with that finality evidence and `account` with the EIP-1186 response. Requests contain `chain_id`, `pool`, `pool_code_hash`, `escrow`, `order_tag` and `nullifier`. Unknown top-level fields, oversized files, incorrect proofs and stale or future evidence are rejected. Successful JSON output is a local verification result, not a transferable proof that another process should trust from a server.

The tests retain official Ethereum consensus vectors with provenance. They cover finality, committee rotation, changed checkpoints/signatures, future/stale evidence, header binding and generated Merkle-Patricia inclusion/non-membership proofs. They do not establish a live Ethereum reservation, authorize a real Zcash payment or complete a swap. No live bootstrap or pool deployment is configured here.

Primary references: [Ethereum weak subjectivity](https://ethereum.org/developers/docs/consensus-mechanisms/pos/weak-subjectivity/), [Helios pinned source](https://github.com/a16z/helios/tree/2af9b41887e069144ee56cd710f3ddca195473f8/ethereum/consensus-core), [EIP-1186](https://eips.ethereum.org/EIPS/eip-1186).
