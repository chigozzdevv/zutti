import argparse
import hashlib
import json
from pathlib import Path
import tempfile

from source_inventory import RpcProxy
from spv_headers import header, write, sync


def export_checkpoint(rpc, height, out):
    if type(height) is not int or not 1 <= height < 500_000_000:
        raise ValueError("invalid checkpoint height")
    if out.exists() or out.is_symlink():
        raise ValueError("checkpoint export already exists")
    info = rpc.call("getblockchaininfo", [])
    if info.get("chain") != "test" or type(info.get("blocks")) is not int or info["blocks"] < height:
        raise ValueError("checkpoint requires a synchronized Testnet node")
    first = max(0, height - 112)
    pinned = rpc.call("getblockhash", [height])
    parent = rpc.call("getblockhash", [first - 1]) if first else "0" * 64
    data = bytearray()
    work = 0
    for current in range(first, height + 1):
        expected = rpc.call("getblockhash", [current])
        raw = header(rpc.call("getblockheader", [expected, False]), expected, parent)
        bits = int.from_bytes(raw[104:108], "little")
        exponent, mantissa = bits >> 24, bits & 0x7fffff
        target = mantissa >> (8 * (3 - exponent)) if exponent <= 3 else mantissa << (8 * (exponent - 3))
        if bits & 0x800000 or not 0 < target < 2**256:
            raise ValueError("invalid checkpoint work target")
        work += 2**256 // (target + 1)
        data.extend(raw)
        parent = expected
    if parent != pinned or rpc.call("getblockhash", [height]) != pinned:
        raise ValueError("checkpoint branch changed during export")
    result = {
        "schema": 1, "network": "testnet", "bootstrap": "checkpoint",
        "height": height, "hash": pinned, "context_first_height": first,
        "context_headers": height - first + 1,
        "headers_sha256": hashlib.sha256(data).hexdigest(),
        "work": work.to_bytes(32, "big").hex(),
        "work_basis": "retained_context",
        "proof_generated": False, "settlement_authority": False,
    }
    with tempfile.TemporaryDirectory(prefix=".spv-checkpoint-", dir=out.parent) as directory:
        stage = Path(directory)
        write(stage / "headers.bin", data)
        write(stage / "export.json", (json.dumps(result, indent=2) + "\n").encode())
        sync(stage)
        stage.rename(out)
        sync(out.parent)
    return result


def main():
    parser = argparse.ArgumentParser(description="Export and pin Testnet checkpoint validation context")
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--cookie", type=Path, required=True)
    parser.add_argument("--container")
    parser.add_argument("--height", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    rpc = RpcProxy(args.endpoint, args.cookie, args.container)
    try:
        print(json.dumps(export_checkpoint(rpc, args.height, args.out)))
    finally:
        rpc.close()


if __name__ == "__main__":
    main()
