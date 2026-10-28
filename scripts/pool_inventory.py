import argparse
import json
from pathlib import Path
import subprocess

from ethereum_evidence import Transport, atomic_save


def cast(*args):
    return subprocess.run(["cast", *args], check=True, capture_output=True,
                          text=True, timeout=10).stdout.strip()


def decode_output(log):
    topics = log["topics"]
    if log.get("removed") or len(topics) != 3:
        raise ValueError("invalid pool output event")
    position = int(topics[1], 16)
    if position >= 2**32 or len(topics[2]) != 66:
        raise ValueError("invalid pool output position or commitment")
    data = bytes.fromhex(log["data"][2:])
    if len(data) < 64 or int.from_bytes(data[:32]) != 32:
        raise ValueError("invalid pool ciphertext ABI offset")
    length = int.from_bytes(data[32:64])
    if not 0 < length <= 4096 or len(data) != 64 + ((length + 31) // 32) * 32:
        raise ValueError("invalid pool ciphertext length")
    if any(data[64 + length:]):
        raise ValueError("invalid pool ciphertext padding")
    return {"position": position, "commitment": topics[2],
            "ciphertext": "0x" + data[64:64 + length].hex()}


def collect(transport, deployment, block_hash):
    if deployment["chain_id"] != 11155111 or int(transport.rpc("eth_chainId", []), 16) != 11155111:
        raise ValueError("Sepolia deployment required")
    block = transport.rpc("eth_getBlockByHash", [block_hash, False])
    if block["hash"].lower() != block_hash.lower():
        raise ValueError("inventory block hash mismatch")
    pool = deployment["pool"]
    pinned = {"blockHash": block_hash, "requireCanonical": True}

    def call(signature):
        value = transport.rpc("eth_call", [{"to": pool, "data": cast("sig", signature)}, pinned])
        if len(value) != 66:
            raise ValueError("invalid pool view result")
        return value

    domain = call("domain()")
    if domain[2:].lower() != deployment["domain"].lower():
        raise ValueError("pool domain mismatch")
    root = call("root()")
    size = int(call("nextLeaf()"), 16)
    if size > 65_536:
        raise ValueError("inventory exceeds the supported replay bound")
    if isinstance(deployment.get('deployment_transactions', {}).get('pool'), dict):
        first = deployment['deployment_transactions']['pool']['height']
    else:
        first = deployment["transactions"]["pool"]["block"]
    if type(first) is not int or first <= 0:
        raise ValueError('invalid pool deployment height')
    last = int(block["number"], 16)
    if last < first or last - first > 64_000:
        raise ValueError("inventory block range exceeds its bound")
    topic = cast("keccak", "OutputInserted(uint32,bytes32,bytes)")
    logs = []
    for start in range(first, last + 1, 1000):
        batch = transport.rpc("eth_getLogs", [{"address": pool, "fromBlock": hex(start),
                                               "toBlock": hex(min(start + 999, last)), "topics": [topic]}])
        if not isinstance(batch, list) or len(logs) + len(batch) > 65_536:
            raise ValueError("inventory log count exceeds its bound")
        for log in batch:
            if log["address"].lower() != pool.lower() or log["topics"][0].lower() != topic.lower():
                raise ValueError("inventory log origin mismatch")
            if not start <= int(log["blockNumber"], 16) <= min(start + 999, last):
                raise ValueError("inventory log range mismatch")
        logs.extend(batch)
    logs.sort(key=lambda entry: tuple(int(entry[field], 16) for field in ("blockNumber", "transactionIndex", "logIndex")))
    outputs = [decode_output(log) for log in logs]
    if len(outputs) != size or any(event["position"] != index for index, event in enumerate(outputs)):
        raise ValueError("incomplete or duplicate pool outputs")
    if transport.rpc("eth_getBlockByNumber", [block["number"], False])["hash"].lower() != block_hash.lower():
        raise ValueError("inventory branch changed during capture")
    return {"schema": 1, "chain_id": 11155111, "pool": pool, "domain": deployment["domain"],
            "root": root, "next_leaf": size, "execution_hash": block_hash,
            "execution_height": last, "outputs": outputs}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--block-hash", required=True)
    parser.add_argument("--rpc", default="https://ethereum-sepolia-rpc.publicnode.com")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    transport = Transport("https://lodestar-sepolia.chainsafe.io", args.rpc)
    value = collect(transport, json.loads(args.deployment.read_bytes()), args.block_hash)
    atomic_save(args.out, value)
    print(json.dumps({"outputs": value["next_leaf"], "root": value["root"],
                      "block": value["execution_height"], "download_bytes": transport.total}))


if __name__ == "__main__":
    main()
