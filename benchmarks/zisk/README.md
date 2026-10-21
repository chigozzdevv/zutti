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
