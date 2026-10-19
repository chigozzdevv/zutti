#!/usr/bin/env python3
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import tempfile
import time


MAX_CYCLES = 1 << 38
BUILD_MANIFEST = Path(__file__).with_name("build.json")
FILES = {
    "job.json": 65536,
    "program.elf": 32 * 1024 * 1024,
    "stdin.bin": 65 * 1024 * 1024,
    "statement.bin": 66560,
    "journal.bin": 66560,
}


def read_regular(path, limit):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= limit:
            raise ValueError(f"invalid input file: {path.name}")
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"input exceeds its bound: {path.name}")
    return data


def snapshot(source, destination):
    for name, limit in FILES.items():
        data = read_regular(source / name, limit)
        with (destination / name).open("xb") as output:
            os.chmod(output.name, 0o600)
            output.write(data)
    job = json.loads((destination / "job.json").read_text())
    if job.get("version") != 1 or job.get("visibility") != "public":
        raise ValueError("only exported public source jobs are accepted")
    if type(job.get("cycles")) is not int or not 0 < job["cycles"] <= MAX_CYCLES:
        raise ValueError("invalid exported cycle bound")
    return job


def command(arguments, deadline, log=None):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("proving window expired")
    environment = os.environ.copy()
    environment["RISC0_DEV_MODE"] = "0"
    process = subprocess.Popen(
        arguments, stdout=log or subprocess.PIPE, stderr=log or subprocess.PIPE,
        start_new_session=True, env=environment,
    )
    try:
        output, error = process.communicate(timeout=remaining)
    except (subprocess.TimeoutExpired, KeyboardInterrupt):
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
        raise TimeoutError("proving process group stopped") from None
    if process.returncode:
        if error:
            raise RuntimeError(error.decode(errors="replace")[-4096:])
        raise RuntimeError(f"worker failed; inspect the retained log (exit {process.returncode})")
    return output


def atomic_json(path, value):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as temporary:
        json.dump(value, temporary, indent=2)
        temporary.write("\n")
        temporary.flush()
        os.fsync(temporary.fileno())
    os.replace(temporary.name, path)


def run(args):
    if not 0 < args.cycle_limit <= MAX_CYCLES or args.segment_po2 not in (20, 21, 22):
        raise ValueError("invalid source limits")
    if not 0 < args.max_seconds <= 86400:
        raise ValueError("max-seconds must be between 1 and 86400")
    os.umask(0o077)
    if os.environ.get("TMPDIR"):
        Path(os.environ["TMPDIR"]).mkdir(parents=True, exist_ok=True)
    args.output.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.max_seconds
    with tempfile.TemporaryDirectory(prefix="source-input-", dir=args.output) as directory:
        job_path = Path(directory)
        job = snapshot(args.job, job_path)
        inspected = json.loads(command([
            args.source_prover, "inspect-job", "--job", str(job_path),
        ], deadline))
        job_id = inspected.get("job_id", "")
        if not re.fullmatch("[0-9a-f]{64}", job_id):
            raise ValueError("invalid source job identity")
        if inspected.get("network") != "testnet" or not inspected.get("source_only"):
            raise ValueError("this worker accepts Testnet source jobs only")
        if inspected.get("image_id") != job.get("image_id"):
            raise ValueError("job manifest does not match the admitted program")
        result = args.output / job_id
        result.mkdir(exist_ok=True)
        with (result / "worker.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            (result / "verified.json").unlink(missing_ok=True)
            proof = result / "source.proof"
            resumed = proof.exists()
            if not resumed:
                if shutil.disk_usage(result).free < args.min_free_gib * 1024**3:
                    raise ValueError("insufficient free disk for the selected worker policy")
                started = time.monotonic()
                hardware = command([
                    "nvidia-smi", "--query-gpu=name,memory.total,driver_version",
                    "--format=csv,noheader",
                ], deadline).decode().strip()
                atomic_json(result / "hardware.json", {
                    "gpu": hardware, "logical_cpus": os.cpu_count(),
                    "build": json.loads(BUILD_MANIFEST.read_text()),
                })
                with (result / "prove.log").open("ab", buffering=0) as log:
                    command([
                        args.source_prover, "prove", "--job", str(job_path),
                        "--out", str(proof), "--prover", args.r0vm,
                        "--cycle-limit", str(args.cycle_limit),
                        "--segment-po2", str(args.segment_po2), "--groth16",
                    ], deadline, log)
                atomic_json(result / "timing.json", {
                    "seconds": time.monotonic() - started,
                    "cycle_limit": args.cycle_limit, "segment_po2": args.segment_po2,
                })
            seal = job_path / "seal.bin"
            command([
                args.source_prover, "verify", "--proof", str(proof),
                "--job", str(job_path), "--seal", str(seal),
            ], deadline)
            seal_bytes = read_regular(seal, 260)
            if len(seal_bytes) != 260:
                raise ValueError("expected a verified Groth16 seal")
            target = result / "seal.bin"
            if target.exists():
                if read_regular(target, 260) != seal_bytes:
                    raise ValueError("retained seal differs from verified proof")
            else:
                with target.open("xb") as output:
                    output.write(seal_bytes)
                    output.flush()
                    os.fsync(output.fileno())
            report = {
                "job_id": job_id, "image_id": inspected["image_id"],
                "network": "testnet", "proof_verified": True,
                "groth16": True, "resumed": resumed,
                "current_chain_admitted": False, "swap_executed": False,
            }
            atomic_json(result / "verified.json", report)
            print(json.dumps(report))


def main():
    parser = argparse.ArgumentParser(description="Prove one public Testnet source job")
    parser.add_argument("--job", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cycle-limit", type=int, required=True)
    parser.add_argument("--max-seconds", type=int, required=True)
    parser.add_argument("--segment-po2", type=int, default=20)
    parser.add_argument("--min-free-gib", type=int, default=100)
    parser.add_argument("--source-prover", default="source-prover")
    parser.add_argument("--r0vm", default="r0vm")
    args = parser.parse_args()
    if args.min_free_gib < 1:
        parser.error("min-free-gib must be positive")
    try:
        def interrupted(_signal, _frame):
            raise KeyboardInterrupt
        signal.signal(signal.SIGTERM, interrupted)
        run(args)
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
