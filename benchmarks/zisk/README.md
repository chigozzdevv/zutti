# ZisK source verification diagnostic

This benchmark verifies one historical Sprout PHGR proof under ZisK. The fixture comes from the Mainnet block 396 vector already used by the source crate's tests. This public test vector does not authenticate Testnet history.

Both guests retain the source verifier's point decoding, subgroup checks, public-input packing, key and five pairing equations. The reference uses arkworks pairings. The candidate converts validated affine points to canonical limbs and evaluates the same equations with ZisK's BN254 pairing library. Neither guest admits a source state or generates a new cryptographic proof.

The isolated Mac setup pins ZisK `v1.3.2-alpha`, commit `1e22630bb7e143c30705a1603ed87cfd3da8c6ec`, and compiler `zisk-4.0.0`. It has no proving keys. Its root contains `bin/`, `toolchain/`, the pinned `source/` tree, `versions.json` and `runtime-paths.json`.

```sh
CARGO_HOME=/path/to/populated/cargo-cache python3 benchmarks/zisk/profile.py \
  --toolchain-root /path/to/isolated/zisk/1.3.2-alpha \
  --work-dir /tmp/zutti-zisk-phgr \
  --trials 3
```

The work directory must not exist. Builds are locked, offline and limited to two compiler workers. Each guest execution has a 60-second timeout and a 250-million-step ceiling; each build has a five-minute timeout. Timeout cleanup stops the process group. A stable linker-script path inside the private work directory allows Cargo to reuse dependencies between the two builds.

The runner checks the valid proof, changed public inputs and a malformed point for each guest. Every result requires successful process exit and an explicit completion marker, so an unfinished execution's empty output cannot pass a rejection case. Valid execution is repeated three times by default. This small corpus is a diagnostic, not a complete verifier audit.

Artifacts include the generated source, source/key/ELF hashes, inputs, logs and `results.json`. Add `--stats` for a bounded profiler run and per-backend statistics CSVs. Step counts and execution times are not proof-generation time, RISC Zero cycles or a bootstrap price. Profiling costs also exclude proof padding and aggregation overhead.

`sprout-phgr.json` records the public fixture and its successful native reference check. `guest.rs` defines the diagnostic input/output framing; `pairing.rs` contains the pairing adapter. `profile.py` prepares and executes both variants, using the committed `Cargo.lock`.

The initial Mac measurements and download accounting are retained in [source-zisk-2026-10-21.json](../source-zisk-2026-10-21.json). The elapsed times compare two implementations running under ZisK, not ZisK against RISC Zero.

## Genuine proof verification

`verify.py` checks the wrapped PLONK fixture shipped with the pinned ZisK release. This is an existing upstream demonstration proof, not a proof of the Zutti guest or Zcash history. `verification-fixture.json` pins the proof, contract sources, program ID and independently downloaded setup key. The runner derives the trusted PLONK key from the fixed Solidity verifier rather than accepting the proof's embedded key as authority.

```sh
python3 benchmarks/zisk/verify.py \
  --toolchain-root /path/to/isolated/zisk/1.3.2-alpha \
  --setup-key /path/to/vadcop_final.verkey.bin \
  --solc /path/to/cached/solc-0.8.30 \
  --work-dir /tmp/zutti-zisk-verification
```

The isolated setup includes its `run` launcher. This check uses cached tools and makes no downloads. It records native CLI verification separately from local EVM verification. The EVM uses the unchanged upstream Solidity verifier and checks a valid proof plus altered public values, program ID, setup root and proof bytes. It does not deploy anything to a network or admit a source checkpoint.

On the tested Mac, native PLONK verification aborted at `Fr_rawToMontgomery() not implemented for macOS`; all five local EVM checks passed. Downloading proving keys would not fix that native arithmetic routine. Neither result demonstrates local proof generation.

Each subprocess has a wall-time limit and a sampled process-group RSS limit. Native verification is limited to 30 seconds/512 MiB; the offline compiler and EVM tests are limited to 90 seconds/768 MiB. The monitor also records `/usr/bin/time -l` child CPU time and peak RSS. Sampling can miss brief memory spikes. Core dumps are disabled, and limit failures terminate the subprocess group. Existing desktop applications are not stopped.

[zisk-verification-2026-10-21.json](../zisk-verification-2026-10-21.json) records the measurements and setup archive sizes checked by HTTP headers. Standard proving setup is about 5.10 GB compressed; the separate PLONK wrapping setup is about 21.91 GB compressed. Expanded storage and proving resource needs were not measured, and those archives were not downloaded.
