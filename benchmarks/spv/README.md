# SPV source prototype

This prototype verifies Zcash headers and transaction inclusion or complete absence within a bounded interval. It relies on miners to enforce transaction validity. Its policy, witness, journal and guest image are separate from the full-validity profile. Existing source-registry and private-pool verifiers do not admit it.

The implementation is in [`crates/source/src/spv`](../../crates/source/src/spv), with a separate [guest](../../crates/prover/methods/source/src/spv.rs). The host runs independently of Zutti's admitted preparation, pool and full-validity programs.

## Measured status

The [source/admission results](results/2026-10-23-admission/manifest.json) and [payment-binding results](results/2026-10-24-binding/manifest.json) use an Apple M1 Pro with bounded local execution and public inputs. No marketplace jobs, paid compute or new chain downloads were used. The [first baseline](results/2026-10-23/manifest.json) remains archived under its original program identity.

| Check | Result |
| --- | --- |
| Native source, work, receipt, preparation and binding tests | 38 passed; clippy reports no warnings |
| Ethereum tests | 28 passed, one existing preparation test skipped; 10 SPV tests use mock proof verification |
| Four-header zkVM execution | 23,101,146 user cycles; genuine genesis predecessor; no new proof |
| Included / absent zkVM execution | 23,459,591 / 23,790,207 user cycles; genuine genesis predecessor; no new proof |
| False absence, missing block and wrong parent | Rejected by native and guest execution |
| Admission zkVM execution | 23,113,855 user cycles; native and guest journals match |
| Version-2 genesis composite receipt | Generated and verified in 159.417 seconds; no added headers |
| Real shielded block coverage | Testnet block 282,757 has 1,634 non-coinbase Sapling transactions; Mainnet block 1,687,107 contains an Orchard transaction; omission fails |
| Prepared Ironwood payment binding | 54,801,665 user cycles; 4.861 seconds execution; native journal matches; no proof generated |
| Changed binding fields | Wrong recipient, amount, expiry, anchor hash and anchor height rejected by native and guest execution |

The [version-2 genesis receipt](fixtures/genesis-v2/receipt.bin) authenticates only the fixed genesis. Its [audit](results/2026-10-23-admission/genesis-proof.json) rejects a wrong image, expected journal, changed journal and changed seal. Genuine non-genesis source/admission/window receipts, Ethereum wrapping and a practical current-chain bootstrap are not demonstrated. The original first-header CPU attempt timed out at 600 seconds; it used the earlier program. Small-input execution times do not predict complete proving time.

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

The demo compares native and guest journals for genesis, four consecutive headers, inclusion and absence. It also checks native and guest rejection of false absence, missing interval coverage and a wrong parent. Execution generates no proof. Supply `--previous` with a matching genesis receipt to execute all continuation cases using genuine predecessor evidence. Continuations supplied only with a predecessor claim are explicitly marked `unresolved_execution_assumption`; they do not authenticate a prefix.

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

`ZUTTISPP` identifies the SPV policy, binding the image, network, confirmation descendants, window bound and explicit supported-height horizon. `ZUTTISPH` version 2 identifies a header-prefix claim and carries rolling difficulty context, cumulative work and the maximum timestamp across every processed header. Version-1 journals cannot be decoded as current prefixes. `ZUTTISPI` identifies guest input. Full-validity encodings reject these messages.

Bootstrap checks the exact fixed network genesis header hash. That identity already fixes the genesis contents; the SPV bootstrap does not recompute its immutable Equihash puzzle. Every added header receives linkage, Equihash, target, expected-difficulty and deterministic timestamp checks. Initial batches can start directly from genesis without an unresolved predecessor. A serialized claim is not a verified receipt or an admitted checkpoint.

`ZUTTISPW` version 2 identifies a window observation. It binds the policy, prefix, exact ending checkpoint identity, salted reservation commitment, candidate head/work and selected fixed output commitment. The transaction ID, salt and branch selector are not journal fields. That output is not an issued USDC note or a private-pool spending authorization.

Inclusion checks a complete block against its verified header and requires the target at or before expiry with confirmation descendants. It can complete before expiry. Absence checks every complete block through inclusive expiry, rejects any present target and requires descendants after expiry. Delayed presentation does not revoke inclusion.

Transaction IDs are derived from serialized transactions and their Merkle root is reconstructed. Omitted/trailing data and duplicate IDs fail. The SPV wrapper also rejects 64-byte legacy encodings, which share the input length and hash construction of Merkle internal nodes. Ambiguous unsupported data fails closed.

## Stable source admission

[`SpvRegistry`](../../contracts/src/SpvRegistry.sol) is a separate, undeployed contract. Its immutable configuration pins the admission image, header-policy identity, confirmation depth, minimum work/height, observation duration, freshness limit and future drift. The test configuration is not a recommended finality policy.

The [admission guest](../../crates/prover/methods/source/src/spv_admission.rs) verifies a genuine header-prefix predecessor, extends it to a proposed stable prefix, then checks exactly the required confirmation descendants. `ZUTTISPA` binds the previous stable identity, new stable identity, head, work, policy and prefix-wide maximum timestamp. It does not authorize tokens or create pool notes.

Any participant can submit verified candidates. A strictly heavier candidate replaces the pending candidate and restarts observation. Finalization requires the delay, fresh source evidence and the exact previously accepted stable ancestry. New proposals must use that stable prefix; old or lower-work proposals fail. The contract checks the maximum timestamp across the proved prefix, rather than the tip alone.

A separately proved heavier fork that contradicts the current stable block can permanently halt admission and make `accepted()` return false. There is no administrator resume or history-root override. This is a safety halt; it does not reverse prior payouts or refund locked funds. Recovery following such a halt remains unresolved.

Source-prefix proving and admission proving are separate programs. Admission currently rechecks the suffix after consuming its predecessor receipt; optimized composition is not measured. Existing preparation, pool and full-validity images remain unchanged. The old pool cannot consume the new journals.

```sh
"$CARGO_TARGET_DIR/debug/zutti-spv-prototype" build --kind admission \
  --guest-target /tmp/spv-guest-target --out /tmp/spv-admission-program

python3 benchmarks/spv/run.py --seconds 180 --rss-mib 4096 \
  --log /tmp/spv-admission.log -- \
  "$CARGO_TARGET_DIR/debug/zutti-spv-prototype" admission \
  --program /tmp/spv-admission-program/program.elf \
  --header-program /tmp/spv-program/program.elf \
  --previous /tmp/spv-genesis/genesis/receipt.bin --out /tmp/spv-admission

forge test --root contracts --offline -j 1 --match-contract SpvRegistryTest
```

The admission command executes against the matching version-2 genesis receipt by default. `--prove` requests a public composite receipt with development mode disabled; no such admission receipt was generated in the recorded run. Ethereum requires genuine wrapping before this program can advance an on-chain registry. Contract tests mock verification and compare the Rust guest's actual journal bytes; their mock seals are not usable proofs.

The retained dense Testnet fixture is a historical Sapling shielding workload. The Orchard fixture is reused Mainnet data for parser compatibility; Zutti's deployment target remains Testnet. Neither proves an Ironwood swap payment, receiver, amount or encrypted memo.

```sh
cargo run --offline --locked --manifest-path benchmarks/spv/Cargo.toml --example inspect -- \
  testnet tests/fixtures/spv/testnet-282757.bin
```

Individual block inspection verifies parsing, the complete transaction root and that header's work. It explicitly reports `chain_authenticated: false` because it supplies no genesis-bound prefix.

## Prepared payment binding

The isolated [binding guest](../../crates/prover/methods/binding/src/main.rs) checks the existing preparation relation directly and links its reservation to the SPV window reservation using the same private transaction ID. Its `ZUTTIBND` journal omits the transaction ID, recipient, amount and memo. The [construction and assumptions](../../docs/payment-window-binding.md) explain the conditional lower bound. The program is not admitted for settlement and does not authenticate an anchor by itself.

```sh
python3 benchmarks/spv/run.py --seconds 300 --rss-mib 4096 \
  --log /tmp/spv-binding-build.log -- \
  "$CARGO_TARGET_DIR/debug/zutti-spv-prototype" build --kind binding \
  --guest-target /tmp/spv-binding-target --out /tmp/spv-binding-program

cargo build --offline --locked --manifest-path benchmarks/spv/Cargo.toml \
  --example payment-binding

python3 benchmarks/spv/run.py --seconds 180 --rss-mib 4096 \
  --log /tmp/spv-binding-execute.log -- \
  "$CARGO_TARGET_DIR/debug/examples/payment-binding" \
  --program /tmp/spv-binding-program/program.elf --out /tmp/spv-binding-execute
```

The example uses generated public Ironwood effects, test-only keys, synthetic terms and a placeholder source-program identity. It broadcasts nothing. Execution accepts the matching journal and rejects five changed fields. Native tests also check policy/profile substitution, ciphertext mutation, valid re-encryption with a different anchor and authorization stability. `--prove` requests a genuine succinct receipt for the positive case, with development mode disabled; it was not used in the recorded run. Execution time is not proof-generation time.

## Limits and remaining work

Inputs are capped at 16 MiB, blocks at 2,000,000 bytes, header batches at 4,096 headers and windows at 128 blocks. Dense windows may exceed the byte cap; composable interval proofs remain necessary before arbitrary windows can be funded reliably.

The generated demo uses real Testnet heights 0 through 4, coinbase-only transaction fixtures and public generated order terms. The inclusion and absence cases exercise source observations, not a prepared shielded swap payment. Its one-descendant setting is not an accepted finality policy. Real private witnesses must not be uploaded to public proving services. Newly generated inputs and receipts use owner-only permissions. The committed genesis fixture contains only public data; Git does not preserve owner-only file modes.

The separate admission prototype below implements candidate comparison, observation, freshness and stable-prefix advancement. It requires proof verification and is not connected to the funded pool. Relevant competing histories must reach it; unseen higher-work chains cannot be ruled out cryptographically. Confirmation descendants and observation time alone do not establish finality.

Absence is conditional on the supplied interval covering every possible inclusion height. The separate binding prototype implements the proposed anchor-hash dependency relation, but no genuine binding receipt or independent review of its lower-bound assumption is complete. A memo containing an anchor is not a consensus-enforced creation timestamp. The window program alone cannot justify a refund.

A current-chain bootstrap, later-upgrade/dense-block measurements, private reservation/settlement composition, Ethereum wrapping and independent funded recovery remain. Execution time is not proving time or a complete bootstrap quote. See the [source-verification review](../../docs/source-verification.md).
