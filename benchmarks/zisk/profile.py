import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import statistics
import subprocess
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
TARGET = "riscv64ima-zisk-zkvm-elf"
VERSION = "1.3.2-alpha"
COMMIT = "1e22630bb7e143c30705a1603ed87cfd3da8c6ec"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_stats(path):
    with path.open(newline="") as stream:
        rows = list(csv.reader(stream))
    steps = next(int(row[1]) for row in rows if row and row[0] == "STEPS")
    cost = next(int(row[2]) for row in rows if len(row) > 2 and row[:2] == ["COST", "TOTAL"])
    return {"steps": steps, "csv_cost": cost}


def execute(command, env, cwd, timeout, log):
    start = time.monotonic()
    with log.open("w") as stream:
        child = subprocess.Popen(command, env=env, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            raise RuntimeError(f"Execution exceeded {timeout}s; retained log: {log}") from None
    if child.returncode:
        raise RuntimeError(f"Command exited {child.returncode}; retained log: {log}")
    return time.monotonic() - start


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise RuntimeError(f"Source layout changed: {old}")
    return text.replace(old, new, 1)


def prepare(work, source):
    upstream = REPO / "crates/source/src/phgr.rs"
    reference = replace_once(upstream.read_text(), '"../parameters/sprout-phgr.vk"', json.dumps(str(REPO / "crates/source/parameters/sprout-phgr.vk")))
    accelerated = replace_once(reference, "ark_bn254::{Bn254,", "ark_bn254::{")
    accelerated = replace_once(accelerated, "pairing::Pairing, ", "")
    accelerated = replace_once(accelerated, "type PreparedG2 = <Bn254 as Pairing>::G2Prepared;", "type PreparedG2 = G2Affine;")
    if accelerated.count("Bn254::multi_pairing") != 5:
        raise RuntimeError("Expected exactly five PHGR pairing equations")
    accelerated = accelerated.replace("Bn254::multi_pairing", "pairing")
    accelerated += "\n" + (HERE / "pairing.rs").read_text()
    (work / "src").mkdir()
    (work / "src/reference.rs").write_text(reference)
    (work / "src/accelerated.rs").write_text(accelerated)
    shutil.copy2(HERE / "guest.rs", work / "src/main.rs")
    shutil.copy2(HERE / "Cargo.lock", work / "Cargo.lock")
    shutil.copy2(source / "ziskbuild/zisk_linker_script.ld", work / "guest.ld")
    runtime = json.dumps(str(source / "ziskos/entrypoint"))
    (work / "Cargo.toml").write_text(f'''[package]
name = "zutti-zisk-source-profile"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[features]
accelerated = []

[dependencies]
ziskos = {{ path = {runtime}, default-features = false }}
ark-bn254 = "=0.5.0"
ark-ec = "=0.5.0"
ark-ff = "=0.5.0"
num-bigint = "=0.4.6"
thiserror = "=2.0.21"

[profile.release]
overflow-checks = true
opt-level = 3
''')
    return {"reference_source_sha256": digest(upstream), "key_sha256": digest(REPO / "crates/source/parameters/sprout-phgr.vk"), "accelerated_source_sha256": digest(work / "src/accelerated.rs"), "linker_sha256": digest(work / "guest.ld")}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--toolchain-root", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--trials", type=int, default=3, choices=range(1, 11))
    parser.add_argument("--timeout", type=int, default=60, choices=range(1, 61))
    parser.add_argument("--stats", action="store_true")
    args = parser.parse_args()
    toolchain = args.toolchain_root.resolve()
    versions = json.loads((toolchain / "versions.json").read_text())
    if versions["zisk"] != "v" + VERSION or versions["zisk_commit"] != COMMIT:
        raise RuntimeError("Unexpected ZisK toolchain profile")
    source = toolchain / "source" / ("zisk-" + COMMIT)
    work = args.work_dir.resolve()
    work.mkdir(mode=0o700, parents=True, exist_ok=False)
    bindings = prepare(work, source)
    env = os.environ.copy()
    for key in ["RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_RISCV64IMA_ZISK_ZKVM_ELF_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"]:
        env.pop(key, None)
    env.update({
        "CARGO_NET_OFFLINE": "true",
        "CARGO_BUILD_JOBS": "2",
        "CARGO_TARGET_DIR": str(work / "target"),
        "RUSTC": str(toolchain / "toolchain/bin/rustc"),
        "DYLD_LIBRARY_PATH": ":".join(json.loads((toolchain / "runtime-paths.json").read_text())),
        "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(["--cfg", "zisk_guest", "-C", f"link-arg=-T{work / 'guest.ld'}"]),
    })
    fixture = json.loads((HERE / "sprout-phgr.json").read_text())
    proof, public = bytes.fromhex(fixture["proof"]), bytes.fromhex(fixture["public"])
    if len(proof) != 296 or len(public) != 272:
        raise RuntimeError("Invalid fixture shape")
    cases = {
        "valid": (proof + public, 1),
        "changed-public": (proof + bytes([public[0] ^ 1]) + public[1:], 0),
        "malformed-point": (bytes([255]) + proof[1:] + public, 0),
    }
    result = {"zisk": VERSION, "rust_toolchain": versions["rust_toolchain"], "proof_generated": False, "source_state_admitted": False, "fixture_sha256": digest(HERE / "sprout-phgr.json"), "bindings": bindings, "builds": [], "runs": []}
    cargo = str(toolchain / "toolchain/bin/cargo")
    emulator = str(toolchain / "bin/ziskemu")
    for backend in ["reference", "accelerated"]:
        print(f"Building {backend} offline", flush=True)
        command = [cargo, "build", "--locked", "--offline", "--release", "--target", TARGET]
        if backend == "accelerated":
            command.extend(["--features", "accelerated"])
        seconds = execute(command, env, work, 300, work / f"build-{backend}.log")
        elf = work / f"{backend}.elf"
        shutil.copy2(work / "target" / TARGET / "release/zutti-zisk-source-profile", elf)
        result["builds"].append({"backend": backend, "wall_seconds": seconds, "elf_sha256": digest(elf)})
        for case, (data, expected) in cases.items():
            input_path = work / f"{case}.bin"
            input_path.write_bytes(data)
            for trial in range(args.trials if case == "valid" else 1):
                name = f"{backend}-{case}-{trial}"
                output = work / f"{name}.bin"
                command = [emulator, "-e", str(elf), "--legacy-inputs", str(input_path), "-o", str(output), "-n", "250000000", "--steps", "-m"]
                log = work / f"{name}.log"
                seconds = execute(command, env, work, args.timeout, log)
                actual = output.read_bytes()
                if actual[:3] != bytes([0x5a, expected, 0xa5]) or any(actual[3:]):
                    raise RuntimeError(f"Unexpected output: {name}")
                steps = re.search(r"STEPS:\s*(\d+)", log.read_text())
                if steps is None:
                    raise RuntimeError(f"Missing execution metrics: {name}")
                row = {"backend": backend, "case": case, "trial": trial, "wall_seconds": seconds, "steps": int(steps[1]), "output_matches": True}
                result["runs"].append(row)
                (work / "results.json").write_text(json.dumps(result, indent=2) + "\n")
                print(json.dumps(row), flush=True)
        if args.stats:
            command = [emulator, "-e", str(elf), "--legacy-inputs", str(work / "valid.bin"), "-n", "250000000", "-X", "--sdk", "--save-stats", str(work / f"{backend}-stats.csv")]
            execute(command, env, work, args.timeout, work / f"{backend}-stats.log")
            stats = read_stats(work / f"{backend}-stats.csv")
            valid = next(row for row in result["runs"] if row["backend"] == backend and row["case"] == "valid")
            if stats["steps"] != valid["steps"]:
                raise RuntimeError(f"Profiler execution differs from validated execution: {backend}")
            result.setdefault("stats", {})[backend] = stats
    result["median_wall_seconds"] = {backend: statistics.median(row["wall_seconds"] for row in result["runs"] if row["backend"] == backend and row["case"] == "valid") for backend in ["reference", "accelerated"]}
    (work / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["median_wall_seconds"]), flush=True)


if __name__ == "__main__":
    main()
