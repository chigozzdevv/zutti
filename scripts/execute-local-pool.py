#!/usr/bin/env python3
import argparse
import json
import subprocess
from pathlib import Path
from urllib.parse import urlparse


def run(arguments, expected=True):
    result = subprocess.run(arguments, capture_output=True, text=True)
    if expected and result.returncode:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc", default="http://127.0.0.1:19547")
    parser.add_argument("--directory", type=Path, default=Path(".data/local-e2e"))
    parser.add_argument("--source", type=Path, default=Path(".data/public-source-update-20261017"))
    args = parser.parse_args()
    if urlparse(args.rpc).hostname not in ("localhost", "127.0.0.1", "::1"):
        raise ValueError("public-key demonstrations require a loopback RPC")
    chain = run(["cast", "chain-id", "--rpc-url", args.rpc]).stdout.strip()
    if chain != "31337":
        raise ValueError("public-key demonstrations require chain 31337")
    deployment = json.loads((args.directory / "deployment.json").read_text())
    owner = deployment["owner"]
    pool = deployment["pool"]
    token = deployment["token"]
    evidence = {"chain_id": 31337, "kind": "generated_public_demo", "transactions": {}}

    def call(address, signature, *values):
        return run(["cast", "call", address, signature, *map(str, values), "--rpc-url", args.rpc]).stdout.strip()

    def send(address, signature, *values):
        result = run(["cast", "send", address, signature, *map(str, values), "--rpc-url", args.rpc, "--from", owner, "--unlocked", "--json"])
        receipt = json.loads(result.stdout)
        if receipt["status"] != "0x1":
            raise RuntimeError("local transaction failed")
        return receipt

    claim = json.loads((args.source / "claim.json").read_text())
    source_result = json.loads((args.source / "result.json").read_text())
    state = "0x" + claim["state"]
    if call(deployment["source"], "headState()(bytes32)") != state:
        evidence["transactions"]["source"] = send(deployment["source"], "update(bytes,bytes32,bytes32,bytes32,uint256)", "0x" + source_result["seal"], "0x" + claim["previous"], state, "0x" + claim["checkpoint"], int(claim["work_be"], 16))
    if call(deployment["source"], "stableCheckpoint()(bytes32)") != "0x" + claim["checkpoint"]:
        raise RuntimeError("source checkpoint mismatch")
    signature = "execute(bytes,(uint8,bytes32,bytes32,bytes32,bytes32,bytes32,uint64,address,bytes32[],(bytes32,bytes)[]),bytes)"
    for name in ("deposit", "transfer", "withdraw"):
        directory = args.directory / "pool" / name
        operation = json.loads((directory / "operation.json").read_text())
        result = json.loads((directory / "result.json").read_text())
        if call(pool, "domain()(bytes32)") != "0x" + operation["domain"]:
            raise RuntimeError("operation domain mismatch")
        nullifiers = "[" + ",".join("0x" + value for value in operation["nullifiers"]) + "]"
        outputs = "[" + ",".join("(0x" + value["commitment"] + ",0x" + value["ciphertext"] + ")" for value in operation["outputs"]) + "]"
        values = [str(operation["kind"]), *["0x" + operation[key] for key in ("root", "checkpoint", "order_tag", "order", "reservation")], str(operation["value"]), "0x" + operation["owner"], nullifiers, outputs]
        operation_tuple = "(" + ",".join(values) + ")"
        already_executed = call(pool, "commitments(bytes32)(bool)", "0x" + operation["outputs"][0]["commitment"]) == "true"
        if not already_executed:
            evidence["transactions"][name] = send(pool, signature, "0x" + result["seal"], operation_tuple, "0x")
        replay = run(["cast", "call", pool, signature, "0x" + result["seal"], operation_tuple, "0x", "--rpc-url", args.rpc, "--from", owner], expected=False)
        if replay.returncode == 0:
            raise RuntimeError(name + " replay unexpectedly succeeded")
    expected = [(pool, "backing()(uint256)", (), 100_000_000), (pool, "nextLeaf()(uint64)", (), 4), (token, "balanceOf(address)(uint256)", (pool,), 100_000_000), (token, "balanceOf(address)(uint256)", (owner,), 100_000_000)]
    for address, function, values, value in expected:
        actual = int(call(address, function, *values).split()[0], 0)
        if actual != value:
            raise RuntimeError(function + " mismatch")
    evidence.update({"source_checkpoint": claim["checkpoint"], "pool_backing": 100_000_000, "pool_token_balance": 100_000_000, "owner_token_balance": 100_000_000, "leaves": 4, "replays_rejected": True})
    (args.directory / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({key: value for key, value in evidence.items() if key != "transactions"}))


if __name__ == "__main__":
    main()
