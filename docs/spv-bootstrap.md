# Recoverable SPV header bootstrap

`zutti-spv-prototype bootstrap` turns consecutive Testnet headers into durable proof jobs. It starts at the fixed Testnet genesis and uses the exact source image and policy selected at initialization. A prepared batch is not verified progress. Only a genuine receipt for its expected journal advances the prefix.

This authenticates the SPV header predicate. It does not validate every transaction, select the canonical chain for Ethereum, prove a payment window or authorize settlement. Registry admission, chain observation, freshness, window evidence and private pool proofs remain separate requirements in the [delivery plan](testnet-delivery.md).

## Export from the existing source node

Run the exporter on the source host beside its existing node. It uses the owner-only RPC cookie and a loopback endpoint. If the node runs in Docker, supply its name with `--container`. No credentials or infrastructure addresses belong in committed configuration.

```sh
python3 scripts/spv_headers.py \
  --endpoint "$SOURCE_RPC" --cookie "$SOURCE_COOKIE" \
  --start-height 0 --count 1 --out "$EXPORTS/genesis"

python3 scripts/spv_headers.py \
  --endpoint "$SOURCE_RPC" --cookie "$SOURCE_COOKIE" \
  --start-height 1 --count 4 --out "$EXPORTS/headers-1-4"
```

Each export atomically publishes `headers.bin` and `manifest.json` in a new directory. The binary contains concatenated 1,487-byte raw headers, without block bodies or a framing prefix. Export is bounded to 4,096 headers per request. It checks exact hashes, parent linkage and the ending block's identity before and after the export. These checks detect inconsistent RPC responses; the exporter does not generate proof or provide consensus authority.

Export the next range from the node when needed. The Rust preparation step independently checks linkage, difficulty, Equihash, timestamps and policy bounds against its own predecessor. Choose batch size from measured proving capacity; the format's maximum is not a throughput claim.

## Initialize and prepare

Build the host and source guest using the [SPV harness](../benchmarks/spv/README.md). Set `SPV` to the built `zutti-spv-prototype` executable. Use the reviewed source-policy values; the small historical test policy is not a deployment policy.

```sh
"$SPV" bootstrap init \
  --state "$STATE" --program "$SOURCE_PROGRAM" \
  --genesis "$EXPORTS/genesis/headers.bin" \
  --confirmations "$CONFIRMATIONS" \
  --window-blocks "$WINDOW_BLOCKS" --valid-through "$VALID_THROUGH"

"$SPV" bootstrap prepare --state "$STATE" \
  --headers "$EXPORTS/headers-1-4/headers.bin"

"$SPV" bootstrap status --state "$STATE"
```

The first batch can prove genesis and its subsequent headers together. Omitting `--headers` prepares genesis alone only when no batch has been verified. Subsequent batches require their exact genuine predecessor. Initialization rejects existing state; use `status` and `prepare` to resume it.

Preparation saves the canonical witness, expected journal and manifest under `jobs/00000000/`, then publishes the complete directory atomically. The manifest binds the batch number, height range, predecessor journal, witness and output hashes. Repeating preparation with identical pending input reuses the batch. A different input is rejected until the pending batch is completed; batch resizing/cancellation is not implemented.

## Prove and accept

Local proof generation must run under explicit elapsed-time, memory and cycle bounds. It can be computationally expensive; this command is not a request to rent compute or spend marketplace funds.

```sh
python3 benchmarks/spv/run.py --seconds 300 --rss-mib 4096 \
  --log "$PROOF_LOG" -- \
  "$SPV" bootstrap prove --state "$STATE" --out "$PROOF_OUTPUT" \
  --cycle-limit 268435456
```

The default output is a succinct receipt. For public header-only proofs, `--public-header-receipt` selects composite output. Composite receipts are not private proof formats or Ethereum seals. This runner never accepts private payment/window witnesses.

`prove` uses the genuine predecessor receipt, verifies the returned proof with development mode disabled and checks the exact expected journal before atomically persisting acceptance. It cannot advance using an unresolved execution assumption. If proving or verification fails, the pending batch remains pending.

An external prover can consume the saved witness, exact program and verified predecessor. Import its result explicitly:

```sh
"$SPV" bootstrap accept --state "$STATE" --batch 0 \
  --receipt "$PROOF_OUTPUT/receipt.bin"
```

Acceptance does not trust a prover's result JSON or claimed height. It verifies the actual receipt against the locally pinned program and journal. Reimporting valid evidence for an already accepted batch is idempotent. Evidence for a different batch, program or journal is rejected.

## Restart and retained state

The initialized state contains `program.elf`, `genesis.bin`, `config.json`, an operating-system lock and the ordered job directories. Keep the complete directory. Every status, preparation or acceptance operation rechecks the program/configuration, batch identities, predecessor continuity and every accepted cryptographic receipt. It retains only the latest receipt in memory. Historical accepted batches do not repeat native Equihash execution during this check; their genuine proofs authenticate the expected output.

There is no separately mutable height cursor. The cursor is the contiguous sequence of verified receipts. A crash before atomic receipt publication leaves a pending batch; a crash after publication leaves evidence that the next operation verifies again. A generated receipt can be imported after restart even if the worker died before acceptance. Incomplete preparation directories are ignored and do not advance progress.

The runner uses exclusive file locking, owner-private initialization/staging, no-overwrite receipt publication and file/directory synchronization. It rejects symlinked artifacts and job gaps. Preserve normal filesystem isolation: this is not protection against a malicious process running as the same account and replacing the entire store.

The verified height is reported separately from pending height. `verified_tip` uses the protocol digest's byte order; the export manifest uses RPC display order. `canonicality_admitted` and `settlement_authority` remain false. This is a recoverable header-job pipeline, not yet an unattended source service: automatic archive traversal, current-chain catch-up, admission generation/submission, fork monitoring and demonstrated sustained proving throughput remain unfinished.

The [retained local run](../benchmarks/spv/results/2026-10-24-bootstrap/manifest.json) generated and accepted a genuine current-image genesis receipt. It then execution-tested four subsequent real headers using that genuine predecessor. The latter operation produced no new receipt, so restart correctly reports only height zero verified and height four pending. The run used the small historical test-policy parameters to test orchestration; it did not select deployment confirmation or freshness settings.
