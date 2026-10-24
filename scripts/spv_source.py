import argparse
import fcntl
import json
import os
import stat
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from source_inventory import RpcProxy
from spv_headers import export


def target(status, info):
    if info.get("chain") != "test" or type(info.get("blocks")) is not int or info["blocks"] < 0:
        raise ValueError("source node must report a Testnet height")
    confirmations = status["confirmations"]
    horizon = status["valid_through"]
    if type(confirmations) is not int or not 1 <= confirmations <= 1000:
        raise ValueError("invalid pinned confirmation depth")
    if type(horizon) is not int or not confirmations < horizon < 500_000_000:
        raise ValueError("source policy leaves no confirmed prefix")
    return max(0, min(info["blocks"], horizon) - confirmations)


def invoke(binary, *args):
    result = subprocess.run([str(binary), *map(str, args)], capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise RuntimeError("SPV command failed: " + result.stderr[-2048:])
    return json.loads(result.stdout.strip().splitlines()[-1])


def status(args):
    return invoke(args.binary, "bootstrap", "status", "--state", args.state)


def emit(value, **fields):
    print(json.dumps({**fields, "source": value, "paid_jobs": 0}), flush=True)


def check_branch(current, info, rpc):
    if info.get("chain") != "test" or type(info.get("blocks")) is not int:
        raise ValueError("source node must report Testnet")
    for prefix in ("verified", "pending"):
        height = current[f"{prefix}_height"]
        if height is None:
            continue
        if info["blocks"] < height:
            raise ValueError("source node is behind the retained header range")
        expected = bytes.fromhex(current[f"{prefix}_tip"])[::-1].hex()
        if rpc.call("getblockhash", [height]) != expected:
            raise ValueError("source branch changed; stop and review competing history")


def run(args, rpc):
    args.work.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(args.state / "worker.lock", os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(descriptor, "w") as lock:
        meta = os.fstat(lock.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
            raise ValueError("source worker lock must be an owner-only regular file")
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        deadline = time.monotonic() + args.max_seconds
        completed = 0
        while time.monotonic() < deadline and completed < args.max_jobs:
            current = status(args)
            info = rpc.call("getblockchaininfo", [])
            check_branch(current, info, rpc)
            height = current["verified_height"]
            if current["pending_batch"] is None:
                end = target(current, info)
                if height is not None and height >= end:
                    if info["blocks"] >= current["valid_through"]:
                        emit(current, state="policy_horizon_reached", target_height=end)
                        return 0
                    emit(current, state="caught_up_to_confirmed_target", target_height=end)
                    if not args.follow:
                        return 0
                    time.sleep(min(args.poll_seconds, max(0, deadline - time.monotonic())))
                    continue
                start = 1 if height is None else height + 1
                count = min(args.batch_size, end - start + 1)
                if count > 0:
                    packet = Path(tempfile.mkdtemp(prefix="headers-", dir=args.work))
                    output = packet / "export"
                    export(rpc, start, count, output)
                    invoke(args.binary, "bootstrap", "prepare", "--state", args.state, "--headers", output / "headers.bin")
                elif height is None:
                    invoke(args.binary, "bootstrap", "prepare", "--state", args.state)
                else:
                    raise RuntimeError("source height cannot advance under this policy")
                current = status(args)
            if not args.prove:
                emit(current, state="prepared")
                return 0
            seconds = min(args.proof_seconds, int(deadline - time.monotonic()))
            if seconds < 1:
                break
            attempt = Path(tempfile.mkdtemp(prefix=f"proof-{current['pending_batch']:08}-", dir=args.work))
            command = [sys.executable, str(Path(__file__).parent.parent / "crates/prover/spv/run.py"),
                       "--seconds", str(seconds), "--rss-mib", str(args.rss_mib), "--log", str(attempt / "prove.log"),
                       "--", str(args.binary), "bootstrap", "prove", "--state", str(args.state),
                       "--out", str(attempt / "result"), "--cycle-limit", str(args.cycle_limit),
                       "--segment-po2", str(args.segment_po2)]
            if not args.succinct:
                command.append("--public-header-receipt")
            emit(current, state="proving", batch=current["pending_batch"], attempt=str(attempt))
            result = subprocess.run(command, check=False)
            after = status(args)
            if result.returncode:
                emit(after, state="proof_incomplete", attempt=str(attempt))
                return 1
            if after["verified_batches"] <= current["verified_batches"]:
                raise RuntimeError("proof worker returned without verified progress")
            completed += 1
            emit(after, state="verified", completed_jobs=completed)
        emit(status(args), state="run_limit", completed_jobs=completed)
        return 0


def main():
    parser = argparse.ArgumentParser(description="Prepare and optionally prove bounded SPV header jobs beside the source node")
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--cookie", type=Path, required=True)
    parser.add_argument("--container")
    parser.add_argument("--prove", action="store_true")
    parser.add_argument("--follow", action="store_true")
    parser.add_argument("--succinct", action="store_true")
    parser.add_argument("--batch-size", type=int, default=1)
    parser.add_argument("--max-jobs", type=int, default=1)
    parser.add_argument("--max-seconds", type=int, default=1800)
    parser.add_argument("--proof-seconds", type=int, default=900)
    parser.add_argument("--rss-mib", type=int, default=4096)
    parser.add_argument("--cycle-limit", type=int, default=268435456)
    parser.add_argument("--segment-po2", type=int, default=19)
    parser.add_argument("--poll-seconds", type=int, default=15)
    args = parser.parse_args()
    if not (1 <= args.batch_size <= 4096 and 1 <= args.max_jobs <= 128 and 1 <= args.max_seconds <= 3600
            and 1 <= args.proof_seconds <= 3600 and 128 <= args.rss_mib <= 8192 and 16 <= args.segment_po2 <= 22
            and 1 <= args.cycle_limit <= 1 << 38 and 1 <= args.poll_seconds <= 60):
        parser.error("invalid workload or resource bounds")
    if args.follow and not args.prove:
        parser.error("following requires explicit local proving")
    if args.state.is_symlink():
        parser.error("source state must not be a symlink")
    args.state = args.state.resolve(strict=True)
    args.binary = args.binary.resolve(strict=True)
    rpc = RpcProxy(args.endpoint, args.cookie, args.container)
    try:
        return run(args, rpc)
    finally:
        rpc.close()


if __name__ == "__main__":
    raise SystemExit(main())
