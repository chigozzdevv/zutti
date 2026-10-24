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

`--segment-po2` selects an execution segment size from 16 through 22; the default is 20. Smaller segments can reduce peak proving memory while increasing segment count and overhead. They preserve the program and statement. One first-header attempt at 20 hit its 4 GiB cap after 629 seconds without producing a receipt; execution time alone did not predict completion.

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

The verified height is reported separately from pending height. `verified_tip` and `pending_tip` use the protocol digest's byte order; the export manifest uses RPC display order. `canonicality_admitted` and `settlement_authority` remain false.

The [retained local run](../benchmarks/spv/results/2026-10-24-bootstrap/manifest.json) generated and accepted a genuine current-image genesis receipt. It then execution-tested four subsequent real headers using that genuine predecessor. The latter operation produced no new receipt, so restart correctly reports only height zero verified and height four pending. The run used the small historical test-policy parameters to test orchestration; it did not select deployment confirmation or freshness settings.

## Follow the existing node

`scripts/spv_source.py` selects consecutive ranges from the existing node and resumes the native store's pending job. By default it prepares one batch and generates no proof:

```sh
python3 scripts/spv_source.py --binary "$SPV" --state "$STATE" --work "$WORK" \
  --endpoint "$SOURCE_RPC" --cookie "$SOURCE_COOKIE" --batch-size 16
```

Add `--prove` to run the bounded local prover. Add `--follow` to wait for subsequent blocks, within `--max-jobs` and `--max-seconds`. Defaults are one header per batch, one job and 1,800 seconds. Each proof is supervised separately with `--proof-seconds`, `--rss-mib`, `--cycle-limit` and `--segment-po2`; defaults are 900 seconds, 4,096 MiB, 268,435,456 cycles and 19. Public composite receipts are the default; `--succinct` selects compression. Large composite receipts can exceed the store's 16 MiB receipt bound, so the input format's header limit is not a safe batch-size recommendation. Overall elapsed-budget checks occur between actions; an active RPC/verification call can run until its own timeout. No marketplace job is submitted.

The worker leaves the configured confirmation suffix available for admission. It checks the node's hashes at both retained verified and pending tips before proving, stops if the node is behind or disagrees, and never replaces an accepted prefix with an RPC-selected fork. At the policy horizon it stops instead of silently extending the supported height. Failed proof attempts leave pending work intact. Its own file lock prevents competing worker instances for the same state.

The worker is implemented but has not caught up public Testnet or demonstrated sustained proving throughput. Its integration test uses a local read-only archive of real historical blocks and the genuine genesis receipt. It checks preparation/restart without claiming that unproved headers or archive responses are authenticated progress. The worker does not submit registry updates or resolve competing histories automatically.

## Initialize and update source admission

An empty registry cannot admit its way through ancient history while enforcing fresh timestamps. `AdmissionWitness::Initialize` solves that composition step: consume a genuine header-prefix receipt already proved from fixed genesis to the chosen stable height, verify its exact confirmation suffix, and emit an advance claim with `previous = 0`. The registry accepts that shape only while its stable prefix is empty. There is no operator-selected recent trust root or reset of an initialized registry.

```sh
"$SPV" prepare-admission --header-program "$SOURCE_PROGRAM" \
  --previous "$STABLE_HEADER_RECEIPT" --confirmations "$CONFIRMATION_HEADERS" \
  --out "$ADMISSION_INPUT"

"$SPV" run-admission --program "$ADMISSION_PROGRAM" \
  --witness "$ADMISSION_INPUT/witness.bin" --previous "$STABLE_HEADER_RECEIPT" \
  --out "$ADMISSION_EXECUTION"
```

Preparation requires a genuine stable-prefix receipt. `run-admission` defaults to execution; `--prove` requests a genuine public composite receipt and must run under the supervisor. For later advances, pass `--stable-headers` to preparation together with the genuine previously accepted stable-prefix receipt. Native validation and guest verification bind that exact predecessor. Subsequent claims name its prefix rather than zero.

The initialization-capable admission image is `e4908d457a2134d5e8b798cf4dd5ebb103fd0e74b31c522f854d15b0dcbcf933`, with ELF SHA-256 `a42d7a685e3ceb28a407b6ae0a97a6d8d260f7c354651f46e9c03b3ce79b5cf8`. The rebuilt header guest retains image `29b1669b9a378dac4ca086083eb4610c8f2b5bf1a5f47d1abe86c6e60c04b104` and its existing ELF hash. A deployment must pin the new admission image; an older admission receipt cannot certify this relation.

Freshness, minimum height/work, competing-history observation, Groth16 wrapping and real Ethereum verification still apply. These commands do not establish a current-chain admission or a funded swap.
