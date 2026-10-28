# Zutti

Private cross-chain RFQ swaps. Send native shielded ZEC and receive USDC in a private Ethereum pool.

## Flow

Request a quote → reserve USDC → verify the reservation → send shielded ZEC → prove the payment → receive private USDC.

Completed Testnet runs include real Zcash payments, private USDC payouts and public withdrawals. [Proofs and transactions](deploy/ethereum/optimized/swap-proof.json).

The browser supports MetaMask Flask, private USDC funding, inventory and transaction history. Fresh browser swap execution is still being connected; completed swaps used the native SDK.

## Setup

Requires Node.js 24+ and pnpm 12.3.4.

```sh
pnpm install --frozen-lockfile
pnpm dev
```

Open **http://127.0.0.1:5173**. The API runs on localhost:3002.

```sh
pnpm build:client
pnpm start:client
```

The production build runs at **http://127.0.0.1:3002**.

For Zcash, use the Zutti Testnet Snap in MetaMask Flask. Build with `pnpm build:wallet`, then run `pnpm serve:wallet`. Building requires Rust 1.93, the `wasm32-unknown-unknown` target, a compatible C compiler and wasm-bindgen 0.2.129. Keep `local:http://localhost:8080` as the Snap ID to recover the same account.

## Structure

- `crates/` — protocol, wallet, source verification, proving and recovery.
- `contracts/` — source registry and private USDC pool.
- `packages/` — SDKs, encrypted RFQ runtime, bindings and Snap.
- `server/` — relay, application API and chain history.
- `client/` — swap, maker and activity pages.
- `deploy/` — deployments and verified proof records.

## Checks

```sh
pnpm build:client
pnpm lint:ts
pnpm test:ts
forge test --root contracts
```

Rust and Foundry are required for the native and contract checks. Tests do not submit paid proving jobs.

## Testnet

The current direction is **ZEC → USDC on Sepolia**. Checkpoint-based SPV trusts the starting checkpoint and relies on miners to enforce Zcash transaction validity. Testnet proving inputs are public; signing and viewing keys stay on the user's device. Reverse swaps and nonpayment refunds are unavailable. Missing evidence can leave a reservation pending.

Use test funds only. See [zutti.md](zutti.md) for the architecture and SDK.
