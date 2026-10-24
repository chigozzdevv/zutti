import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile

from source_inventory import RpcProxy

HEADER_BYTES = 1487
MAX_HEADERS = 4096
HASH = re.compile(r"[0-9a-f]{64}\Z")


def header(raw, expected_hash, parent):
    if not isinstance(raw, str) or not 2 * HEADER_BYTES <= len(raw) <= 4_000_000:
        raise ValueError("invalid serialized source block size")
    if not isinstance(expected_hash, str) or not HASH.fullmatch(expected_hash):
        raise ValueError("invalid source block hash")
    value = bytes.fromhex(raw[:2 * HEADER_BYTES])
    if len(value) != HEADER_BYTES or value[140:143] != b"\xfd\x40\x05":
        raise ValueError("unsupported source header encoding")
    actual = hashlib.sha256(hashlib.sha256(value).digest()).digest()[::-1].hex()
    if actual != expected_hash or value[4:36][::-1].hex() != parent:
        raise ValueError("source header identity or parent mismatch")
    return value


def write(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())


def sync(path):
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def export(rpc, start, count, out):
    if type(start) is not int or type(count) is not int or not 0 <= start < 500_000_000:
        raise ValueError("invalid header range")
    if not 1 <= count <= MAX_HEADERS or start + count >= 500_000_000:
        raise ValueError("invalid header batch size")
    if out.exists() or out.is_symlink():
        raise ValueError("header export already exists")
    info = rpc.call("getblockchaininfo", [])
    if info["chain"] != "test" or info["blocks"] < start + count - 1:
        raise ValueError("source node is not Testnet or has not reached this range")
    end = start + count - 1
    pinned = rpc.call("getblockhash", [end])
    parent = rpc.call("getblockhash", [start - 1]) if start else "0" * 64
    before = parent
    values = bytearray()
    for height in range(start, end + 1):
        expected = rpc.call("getblockhash", [height])
        values.extend(header(rpc.call("getblock", [expected, 0]), expected, parent))
        parent = expected
    if parent != pinned or rpc.call("getblockhash", [end]) != pinned:
        raise ValueError("source branch changed during header export")
    manifest = {
        "schema": 1, "network": "testnet", "first_height": start, "last_height": end,
        "parent_hash": before, "last_hash": pinned,
        "headers_sha256": hashlib.sha256(values).hexdigest(), "header_bytes": HEADER_BYTES,
        "proof_generated": False, "canonicality_admitted": False, "settlement_authority": False,
    }
    with tempfile.TemporaryDirectory(prefix=".spv-headers-", dir=out.parent) as directory:
        stage = Path(directory)
        write(stage / "headers.bin", values)
        write(stage / "manifest.json", json.dumps(manifest, indent=2).encode() + b"\n")
        sync(stage)
        os.rename(stage, out)
        sync(out.parent)
    return manifest


def main():
    parser = argparse.ArgumentParser(description="Export bounded raw Testnet headers from the local source node")
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--cookie", type=Path, required=True)
    parser.add_argument("--container")
    parser.add_argument("--start-height", type=int, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    rpc = RpcProxy(args.endpoint, args.cookie, args.container)
    try:
        print(json.dumps(export(rpc, args.start_height, args.count, args.out)))
    finally:
        rpc.close()


if __name__ == "__main__":
    main()
