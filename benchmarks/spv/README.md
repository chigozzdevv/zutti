# SPV source prototype

This prototype verifies Zcash headers and transaction inclusion or complete absence within a bounded interval. It relies on miners to enforce transaction validity. Its policy, witness, journal and guest image are separate from the full-validity profile. Existing source-registry and private-pool verifiers do not admit it.

The implementation is in [`crates/source/src/spv`](../../crates/source/src/spv), with a separate [guest](../../crates/prover/methods/source/src/spv.rs). The host runs independently of Zutti's admitted preparation, pool and full-validity programs.

## Measured status

The [2026-10-23 results](results/2026-10-23/manifest.json) use an Apple M1 Pro with two proving threads and generated public inputs. No marketplace jobs or paid compute were used.

| Check | Result |
| --- | --- |
| Native source, work and receipt tests | 20 passed, including verification of the retained genuine receipt |
| Resource supervisor tests | Success, memory-limit reporting and termination of a resistant descendant passed |
| Four-header zkVM execution | 23,101,005 user cycles; 0.526 seconds; no proof |
| Included / absent zkVM execution | 23,454,306 / 23,784,922 user cycles; no proof |
| False absence, missing block and wrong parent | Rejected by both native and guest execution |
| Genuine genesis-only composite receipt | Generated and verified in 142.216 seconds; no added headers |
| Genesis plus the first header | No verified receipt before the 600-second limit |

The [genesis receipt](fixtures/genesis/receipt.bin) is a cryptographically verified public-header fixture. Its [audit](results/2026-10-23/genesis-proof.json) rejects a wrong image, wrong expected journal, changed journal bytes and changed seal. It authenticates only the fixed genesis. Genuine inclusion, absence, recursive continuation, Ethereum wrapping and a practical current-chain bootstrap have not been demonstrated by this run. Small-input execution times do not predict complete proving time.

## Build and check

Set `CARGO_HOME` to a populated cache and `CARGO_TARGET_DIR` to a host target directory. Dependencies are locked and offline.

```sh
cargo test --offline --locked --manifest-path benchmarks/spv/Cargo.toml --tests
cargo clippy --offline --locked --manifest-path benchmarks/spv/Cargo.toml --all-targets -- -D warnings
cargo build --offline --locked --manifest-path benchmarks/spv/Cargo.toml
python3 -m unittest discover -s benchmarks/spv -p 'test_*.py'

python3 benchmarks/spv/run.py --seconds 600 --rss-mib 4096 \
  --log /tmp/spv-build.log -- \
  "$CARGO_TARGET_DIR/debug/zutti-spv-prototype" build \
  --guest-target /tmp/spv-guest-target --out /tmp/spv-program
```

Reuse an existing guest target to avoid recompiling cached dependencies. Output and log paths must be new. The supervisor bounds elapsed time and sampled process-group memory, limits threads and blocks external HTTP downloads through child proxy settings. It terminates the process group on limit or monitor failure. Sampling can miss brief memory spikes. It does not rent machines or submit marketplace jobs.

## Execute or prove

```sh
python3 benchmarks/spv/run.py --seconds 180 --rss-mib 4096 \
  --log /tmp/spv-execute.log -- \
  "$CARGO_TARGET_DIR/debug/zutti-spv-prototype" demo \
  --program /tmp/spv-program/program.elf --out /tmp/spv-execute
```

The demo compares native and guest journals for genesis, four consecutive headers, inclusion and absence. It also checks native and guest rejection of false absence, missing interval coverage and a wrong parent. Execution generates no proof. Continuations supplied only with a predecessor claim are explicitly marked `unresolved_execution_assumption`; they do not authenticate a prefix.

The default cycle guard is 268,435,456. `--cycle-limit` can explicitly request up to 274,877,906,944 cycles without changing the statement. The supervisor's time and memory bounds apply separately. Neither limit is a promise of proof completion or a fundamental proving ceiling.

Generate a genuine public-header receipt from genesis through the first block:

```sh
python3 benchmarks/spv/run.py --seconds 600 --rss-mib 4096 \
  --log /tmp/spv-first-header.log -- \
  "$CARGO_TARGET_DIR/debug/zutti-spv-prototype" demo --case first-header --prove \
  --public-header-receipt --program /tmp/spv-program/program.elf --out /tmp/spv-first-header
```

`--public-header-receipt` selects a larger composite receipt and is restricted to public header inputs. Private window witnesses cannot use this option. Receipt verification remains cryptographic, with development mode disabled; no privacy or Ethereum-wrapping claim is made for that format.

Without that flag, `--prove` requests a succinct receipt. For a genesis-only receipt use `--case genesis`. A genuine continuation or window receipt uses `--case headers`, `--case included` or `--case absent`, with `--prove --previous /tmp/spv-genesis/genesis/receipt.bin`. Both host verification and guest composition bind the exact predecessor and program. Proving never substitutes an unresolved claim for genuine evidence. Neither format is an Ethereum Groth16 seal.

```sh
"$CARGO_TARGET_DIR/debug/zutti-spv-prototype" verify \
  --program /tmp/spv-program/program.elf \
  --receipt /tmp/spv-first-header/first-header/receipt.bin \
  --journal /tmp/spv-first-header/first-header/journal.bin

"$CARGO_TARGET_DIR/debug/zutti-spv-prototype" audit \
  --program /tmp/spv-program/program.elf \
  --receipt /tmp/spv-first-header/first-header/receipt.bin --out /tmp/spv-audit.json
```

The audit checks a genuine receipt and rejects a wrong image, a wrong expected journal, changed journal bytes and a changed proof seal. A limit failure is a failed measurement; it does not authorize more spending.

## Statements and coverage

`ZUTTISPP` identifies the SPV policy, binding the image, network, confirmation descendants, window bound and explicit supported-height horizon. `ZUTTISPH` identifies a header-prefix claim and carries rolling difficulty context and cumulative work. `ZUTTISPI` identifies guest input. Full-validity encodings reject these messages.

Bootstrap checks the exact fixed network genesis header hash. That identity already fixes the genesis contents; the SPV bootstrap does not recompute its immutable Equihash puzzle. Every added header receives linkage, Equihash, target, expected-difficulty and deterministic timestamp checks. Initial batches can start directly from genesis without an unresolved predecessor. A serialized claim is not a verified receipt or an admitted checkpoint.

`ZUTTISPW` identifies a window observation. It binds the policy, prefix, salted reservation commitment, candidate head/work and selected fixed output commitment. The transaction ID, salt and branch selector are not journal fields. That output is not an issued USDC note or a private-pool spending authorization.

Inclusion checks a complete block against its verified header and requires the target at or before expiry with confirmation descendants. It can complete before expiry. Absence checks every complete block through inclusive expiry, rejects any present target and requires descendants after expiry. Delayed presentation does not revoke inclusion.

Transaction IDs are derived from serialized transactions and their Merkle root is reconstructed. Omitted/trailing data and duplicate IDs fail. The SPV wrapper also rejects 64-byte legacy encodings, which share the input length and hash construction of Merkle internal nodes. Ambiguous unsupported data fails closed.

## Limits and remaining work

Inputs are capped at 16 MiB, blocks at 2,000,000 bytes, header batches at 4,096 headers and windows at 128 blocks. Dense windows may exceed the byte cap; composable interval proofs remain necessary before arbitrary windows can be funded reliably.

The generated demo uses real Testnet heights 0 through 4, coinbase-only transaction fixtures and public generated order terms. The inclusion and absence cases exercise source observations, not a prepared shielded swap payment. Its one-descendant setting is not an accepted finality policy. Real private witnesses must not be uploaded to public proving services. Newly generated inputs and receipts use owner-only permissions. The committed genesis fixture contains only public data; Git does not preserve owner-only file modes.

The prototype does not decide canonical fork choice, stable-prefix advancement, observation/freshness policy or wall-clock future-header admission. Confirmation descendants alone do not establish finality.

Absence is conditional on the supplied interval covering every possible inclusion height. Preparation and cryptographic birth after the lower boundary are not proved here. A memo containing an anchor is not a consensus-enforced creation timestamp. This must be resolved before a window observation can authorize a refund.

A current-chain bootstrap, later-upgrade/dense-block measurements, private reservation/settlement composition, Ethereum wrapping and independent funded recovery remain. Execution time is not proving time or a complete bootstrap quote. See the [source-verification review](../../docs/source-verification.md).
