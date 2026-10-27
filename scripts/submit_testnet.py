import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.request


def save(path, value):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
        os.chmod(output.name, 0o600)
        json.dump(value, output, indent=2)
        output.flush()
        os.fsync(output.fileno())
    os.replace(output.name, path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


class Rpc:
    def __init__(self, url):
        self.url = url

    def call(self, method, params):
        request = urllib.request.Request(
            self.url,
            json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode(),
            {"Content-Type": "application/json", "User-Agent": "Zutti/1.0"},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            result = json.load(response)
        if "error" in result or "result" not in result:
            raise RuntimeError(f"{method} failed: {result.get('error')}")
        return result["result"]


def command(arguments):
    result = subprocess.run(arguments, capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise RuntimeError(result.stderr[-2000:])
    return result.stdout.strip()


def confirmed(rpc, path, record):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        receipt = rpc.call("eth_getTransactionReceipt", [record["hash"]])
        if receipt:
            record["receipt"] = receipt
            record["state"] = "confirmed" if int(receipt["status"], 16) == 1 else "failed"
            save(path, record)
            if record["state"] != "confirmed":
                raise RuntimeError(f"Transaction failed: {record['hash']}")
            return record
        time.sleep(4)
    raise RuntimeError("Receipt pending; repeat this exact operation to collect without resubmitting")


def submit(args):
    transaction = json.loads(args.transaction.read_text())
    if transaction.get("chain_id") not in (11155111, "11155111"):
        raise ValueError("Sepolia transaction required")
    if transaction.get("value", "0x0") != "0x0":
        raise ValueError("ETH transfers are not supported")
    data = transaction["data"]
    if not isinstance(data, str) or not re.fullmatch(r"0x(?:[0-9a-fA-F]{2})+", data):
        raise ValueError("Invalid transaction data")
    to = transaction.get("to")
    if to is not None and (not re.fullmatch(r"0x[0-9a-fA-F]{40}", to) or int(to, 16) == 0):
        raise ValueError("Invalid destination")
    if args.max_fee_wei <= 0 or args.priority_fee_wei < 0 or args.priority_fee_wei > args.max_fee_wei:
        raise ValueError("Invalid gas fee bounds")
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", args.name):
        raise ValueError("Invalid operation name")
    rpc = Rpc(args.rpc_url)
    if int(rpc.call("eth_chainId", []), 16) != 11155111:
        raise ValueError("RPC is not Sepolia")
    sender = command(["cast", "wallet", "address", "--keystore", str(args.keystore),
                      "--password-file", str(args.password_file)])
    if not re.fullmatch(r"0x[0-9a-fA-F]{40}", sender):
        raise ValueError("Invalid signing account")
    request = {"from": sender, "data": data, "value": "0x0"}
    if to is not None:
        request["to"] = to
    identity = hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest()
    args.out.mkdir(mode=0o700, parents=False, exist_ok=True)
    with (args.out / "lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        path = args.out / f"{args.name}.json"
        if path.exists():
            record = json.loads(path.read_text())
            if record["request_sha256"] != identity or record["sender"].lower() != sender.lower():
                raise ValueError("Operation was already used for a different transaction")
            if "hash" not in record:
                raise RuntimeError("Prior send outcome is unknown; inspect the nonce before continuing")
            return confirmed(rpc, path, record)
        reserved = 0
        for existing in args.out.glob("*.json"):
            record = json.loads(existing.read_text())
            reserved += record["gas_limit"] * record["max_fee_wei"]
            if record["state"] not in ("confirmed", "failed"):
                raise RuntimeError("Resolve the previous operation before sending another transaction")
        estimate = int(rpc.call("eth_estimateGas", [request]), 16)
        block_limit = int(rpc.call("eth_getBlockByNumber", ["latest", False])["gasLimit"], 16)
        gas = min((estimate * 115 + 99) // 100, args.max_gas, block_limit)
        if gas < estimate or reserved + gas * args.max_fee_wei > args.budget_wei:
            raise ValueError("Transaction exceeds configured gas or cumulative fee budget")
        if int(rpc.call("eth_getBalance", [sender, "latest"]), 16) < gas * args.max_fee_wei:
            raise ValueError("Insufficient Testnet gas balance")
        request["gas"] = hex(gas)
        rpc.call("eth_call", [request, "latest"])
        nonce = int(rpc.call("eth_getTransactionCount", [sender, "pending"]), 16)
        record = {"chain_id": 11155111, "sender": sender, "nonce": nonce,
                  "request_sha256": identity, "gas_estimate": estimate, "gas_limit": gas,
                  "max_fee_wei": args.max_fee_wei, "state": "sending",
                  "created_at": int(time.time()), "automatic_resubmission": False}
        save(path, record)
        arguments = ["cast", "send", "--async", "--rpc-url", args.rpc_url, "--chain", "11155111",
                     "--keystore", str(args.keystore), "--password-file", str(args.password_file),
                     "--gas-limit", str(gas), "--gas-price", str(args.max_fee_wei),
                     "--priority-gas-price", str(args.priority_fee_wei), "--nonce", str(nonce)]
        arguments += ["--create", data] if to is None else [to, data]
        tx_hash = command(arguments)
        if not re.fullmatch(r"0x[0-9a-fA-F]{64}", tx_hash):
            raise RuntimeError("Send returned an unexpected result; inspect the retained operation")
        record.update({"hash": tx_hash, "state": "submitted"})
        save(path, record)
        return confirmed(rpc, path, record)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--transaction", type=Path, required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--keystore", type=Path, required=True)
    parser.add_argument("--password-file", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rpc-url", default="https://ethereum-sepolia-rpc.publicnode.com")
    parser.add_argument("--max-fee-wei", type=int, default=1_000_000)
    parser.add_argument("--priority-fee-wei", type=int, default=1000)
    parser.add_argument("--max-gas", type=int, default=32_000_000)
    parser.add_argument("--budget-wei", type=int, default=20_000_000_000_000_000)
    args = parser.parse_args()
    record = submit(args)
    receipt = record["receipt"]
    print(json.dumps({"name": args.name, "hash": record["hash"], "chain_id": 11155111,
                      "block": int(receipt["blockNumber"], 16), "status": record["state"],
                      "contract_address": receipt.get("contractAddress"),
                      "gas_used": int(receipt["gasUsed"], 16), "finality_verified": False}))


if __name__ == "__main__":
    main()
