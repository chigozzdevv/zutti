#!/usr/bin/env python3
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
from source_rpc import Rpc


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc", required=True)
    parser.add_argument("--rpc-cookie", required=True, type=Path)
    parser.add_argument("--replay", required=True, type=Path)
    parser.add_argument("--programs", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--network", required=True, choices=["mainnet", "testnet"])
    parser.add_argument("--through-height", required=True, type=int)
    parser.add_argument("--index-start", default=0, type=int)
    parser.add_argument("--confirmations", default=3, type=int)
    parser.add_argument("--batch-size", default=128, type=int)
    parser.add_argument("--sample-start", action="append", default=[], type=int)
    parser.add_argument("--sample-size", default=16, type=int)
    return parser.parse_args()


def main(args):
    os.umask(0o077)
    rpc = Rpc(args.rpc, args.rpc_cookie)
    info = rpc.call("getblockchaininfo", [])
    if info["chain"] != ("test" if args.network == "testnet" else "main"):
        raise ValueError("RPC chain differs from the selected network")
    if not 0 <= args.through_height <= info["blocks"]:
        raise ValueError("requested replay exceeds the available source history")
    args.out.mkdir(mode=0o700, parents=True, exist_ok=True)
    genesis = bytes.fromhex(rpc.call("getblock", ["0", 0]))
    genesis_path = args.out / "genesis.bin"
    try:
        with genesis_path.open("xb") as out:
            out.write(genesis)
    except FileExistsError:
        if genesis_path.read_bytes() != genesis:
            raise ValueError("retained genesis differs from the node") from None
    images = json.loads((args.programs / "programs.json").read_text())["images"]
    command = [str(args.replay), "--network", args.network, "--database", str(args.database),
               "--genesis", str(genesis_path), "--source-program", images["source"],
               "--registry-program", images["source-update"], "--index-start", str(args.index_start),
               "--confirmations", str(args.confirmations), "--through-height", str(args.through_height),
               "--batch-size", str(args.batch_size), "--sample-size", str(args.sample_size),
               "--samples", str(args.out / "samples")]
    for height in args.sample_start:
        command.extend(["--sample-start", str(height)])
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=False)
    try:
        ready_line = child.stdout.readline()
        if not ready_line:
            raise RuntimeError("native replay failed before opening its frontier")
        ready = json.loads(ready_line)
        print(json.dumps(ready), flush=True)
        start = ready["native_height"]
        if bytes.fromhex(rpc.call("getblockhash", [start]))[::-1].hex() != ready["tip"]:
            raise RuntimeError("node branch differs from the retained replay frontier")
        for height in range(start + 1, args.through_height + 1):
            raw = bytes.fromhex(rpc.call("getblock", [str(height), 0]))
            if not 0 < len(raw) <= 2_000_000:
                raise RuntimeError("serialized block exceeds its frame bound")
            child.stdin.write(struct.pack("<I", len(raw)))
            child.stdin.write(raw)
            child.stdin.flush()
        child.stdin.close()
        remaining = child.stdout.read(65_537)
        if len(remaining) > 65_536 or child.wait() != 0:
            raise RuntimeError("native replay failed; its committed frontier remains available")
        print(remaining.decode().strip(), flush=True)
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait()
        rpc.connection.close()


if __name__ == "__main__":
    try:
        main(arguments())
    except Exception as error:
        print(f"source profiling failed: {error}", file=sys.stderr)
        sys.exit(1)
