import argparse
import hashlib
from http.client import RemoteDisconnected
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
import urllib.request


MAX_RESPONSE = 2_097_152
MAX_TOTAL = 16_777_216


class Transport:
    def __init__(self, beacon, execution, allow_http_beacon=False):
        for index, url in enumerate((beacon, execution)):
            parsed = urlsplit(url)
            schemes = {"https", "http"} if index == 0 and allow_http_beacon else {"https"}
            if (parsed.scheme not in schemes or not parsed.hostname or parsed.username
                    or parsed.password or parsed.fragment or parsed.query):
                raise ValueError("public HTTPS endpoints are required")
        self.beacon = beacon.rstrip("/")
        self.execution = execution
        self.total = 0
        self.calls = 0

    def request(self, url, payload=None):
        request = urllib.request.Request(
            url, None if payload is None else json.dumps(payload).encode(),
            {"Accept": "application/json", "Content-Type": "application/json", "User-Agent": "Zutti/1.0"},
        )
        methods = {"eth_chainId", "eth_getBlockByNumber", "eth_getBlockByHash", "eth_getProof"}
        calls = payload if isinstance(payload, list) else [payload]
        readonly = payload is None or bool(calls) and all(
            isinstance(call, dict) and call.get("method") in methods for call in calls)
        for attempt in range(3 if readonly else 1):
            self.calls += 1
            if self.calls > 72:
                raise ValueError("evidence request limit exceeded")
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    raw = response.read(min(MAX_RESPONSE, MAX_TOTAL - self.total) + 1)
                break
            except (URLError, RemoteDisconnected, ConnectionError, TimeoutError) as error:
                if (not readonly or attempt == 2
                        or isinstance(error, HTTPError) and error.code not in (429, 502, 503, 504)):
                    raise
                delay = 3 * (attempt + 1)
                if isinstance(error, HTTPError):
                    retry = error.headers.get("Retry-After")
                    if retry and retry.isdecimal():
                        delay = max(delay, int(retry))
                    error.close()
                if delay > 30:
                    raise
                time.sleep(delay)
        self.total += len(raw)
        if len(raw) > MAX_RESPONSE or self.total > MAX_TOTAL:
            raise ValueError("evidence exceeds its download bound")
        return json.loads(raw)

    def rest(self, path):
        return self.request(self.beacon + path)["data"]

    def rpc(self, method, params):
        value = self.request(self.execution, {"jsonrpc": "2.0", "id": self.calls + 1,
                                              "method": method, "params": params})
        if value.get("error") or value.get("result") is None:
            raise ValueError(f"execution RPC rejected {method}")
        return value["result"]

    def blocks(self, heights):
        payload = [{"jsonrpc": "2.0", "id": index, "method": "eth_getBlockByNumber",
                    "params": [hex(height), False]} for index, height in enumerate(heights)]
        values = self.request(self.execution, payload)
        if not isinstance(values, list) or len(values) != len(payload):
            raise ValueError("incomplete execution header batch")
        results = {}
        for value in values:
            identifier = value.get("id")
            if identifier in results or identifier not in range(len(payload)) or value.get("error") or not value.get("result"):
                raise ValueError("invalid execution header batch")
            results[identifier] = value["result"]
        return [results[index] for index in range(len(payload))]


def request_digest(requests):
    return hashlib.sha256(json.dumps(requests, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def capture(transport, requests):
    if int(transport.rpc("eth_chainId", []), 16) != 11155111:
        raise ValueError("Sepolia RPC required")
    block = transport.rpc("eth_getBlockByNumber", ["latest", False])
    proofs = {}
    for name, request in requests.items():
        if name not in {"pool", "source", "account"} or len(request["keys"]) > 16:
            raise ValueError("unsupported proof request")
        proofs[name] = transport.rpc("eth_getProof", [request["address"], request["keys"],
                                                     {"blockHash": block["hash"], "requireCanonical": True}])
    return {"schema": 1, "request_digest": request_digest(requests), "execution": block,
            "proofs": proofs, "finality_verified": False}


def collect(transport, config, requests, captured=None):
    if config["chain_id"] != 11155111 or int(transport.rpc("eth_chainId", []), 16) != 11155111:
        raise ValueError("Sepolia configuration and RPC required")
    bootstrap = transport.rest("/eth/v1/beacon/light_client/bootstrap/" + config["checkpoint"])
    finality = transport.rest("/eth/v1/beacon/light_client/finality_update")
    bootstrap_period = int(bootstrap["header"]["beacon"]["slot"]) // 8192
    signature_period = int(finality["signature_slot"]) // 8192
    count = signature_period - bootstrap_period
    if count < 0 or count > 64:
        raise ValueError("checkpoint is outside the supported committee window")
    updates = []
    if count:
        values = transport.request(
            transport.beacon + f"/eth/v1/beacon/light_client/updates?start_period={bootstrap_period}&count={count}"
        )
        if not isinstance(values, list) or len(values) != count:
            raise ValueError("incomplete committee updates")
        updates = [value["data"] for value in values]
    header = finality["finalized_header"]
    block_hash = header.get("execution_block_hash") or header["execution"]["block_hash"]
    execution = transport.rpc("eth_getBlockByHash", [block_hash, False])
    if execution["hash"].lower() != block_hash.lower():
        raise ValueError("execution block hash mismatch")
    ancestry = []
    if captured is not None:
        if captured["schema"] != 1 or captured["request_digest"] != request_digest(requests):
            raise ValueError("captured proof request mismatch")
        start = int(captured["execution"]["number"], 16)
        end = int(execution["number"], 16)
        if start > end:
            raise ValueError("captured state is awaiting finality; retain it and retry verification later")
        if end - start > 128:
            raise ValueError("captured state exceeds its ancestry bound")
        for first in range(start + 1, end + 1, 32):
            ancestry.extend(transport.blocks(range(first, min(first + 32, end + 1))))
        execution = captured["execution"]
    evidence = {"consensus": {"bootstrap": bootstrap, "updates": updates,
                               "finality": finality, "execution": execution, "ancestry": ancestry}}
    for name, request in requests.items():
        if name not in {"pool", "source", "account"} or len(request["keys"]) > 16:
            raise ValueError("unsupported proof request")
        proof = captured["proofs"][name] if captured is not None else transport.rpc(
            "eth_getProof", [request["address"], request["keys"],
                             {"blockHash": block_hash, "requireCanonical": True}])
        if proof["address"].lower() != request["address"].lower():
            raise ValueError("account proof address mismatch")
        evidence[name] = proof
    return evidence


def atomic_save(path, value):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as out:
        temporary = Path(out.name)
        try:
            os.chmod(temporary, 0o600)
            json.dump(value, out, separators=(",", ":"))
            out.flush()
            os.fsync(out.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        os.link(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        temporary.unlink(missing_ok=True)


def native(binary, args):
    result = subprocess.run([str(binary), *args], check=True, capture_output=True, text=True, timeout=60)
    if len(result.stdout) > 65_536:
        raise ValueError("native result exceeds its bound")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--kind", choices=["finality", "note", "reservation", "inventory", "ready"], required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--request", type=Path)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--beacon", default="https://lodestar-sepolia.chainsafe.io")
    parser.add_argument("--allow-http-beacon", action="store_true")
    parser.add_argument("--rpc", default="https://ethereum-sepolia-rpc.publicnode.com")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--capture", type=Path)
    parser.add_argument("--capture-only", action="store_true")
    args = parser.parse_args()
    if args.out.exists() or not args.native.is_absolute():
        raise ValueError("new output and absolute native executable required")
    config = json.loads(args.config.read_bytes())
    requests = {}
    if args.kind != "finality":
        if args.request is None:
            raise ValueError("verification request required")
        requests = native(args.native, ["proof-keys", "--kind", args.kind, "--request", str(args.request)])
    transport = Transport(args.beacon, args.rpc, args.allow_http_beacon)
    if args.capture_only:
        if not requests or args.capture is not None:
            raise ValueError("capture requires one storage request and no existing capture")
        value = capture(transport, requests)
        atomic_save(args.out, value)
        print(json.dumps({"captured_height": int(value["execution"]["number"],16),
                          "finality_verified":False,"download_bytes":transport.total,"capture":str(args.out)}))
        return
    captured = None
    if args.capture is not None:
        if args.capture.stat().st_size > MAX_TOTAL:
            raise ValueError("capture exceeds its size bound")
        captured = json.loads(args.capture.read_bytes())
    evidence = collect(transport, config, requests, captured)
    if args.kind == "finality":
        evidence = evidence["consensus"]
    with tempfile.TemporaryDirectory(prefix=".zutti-evidence-", dir=args.out.parent) as directory:
        staged = Path(directory) / "evidence.json"
        atomic_save(staged, evidence)
        command = [args.kind, "--config", str(args.config), "--evidence", str(staged)]
        if args.kind != "finality":
            command.extend(["--request", str(args.request)])
        verified = native(args.native, command)
        atomic_save(args.out, evidence)
    if "header" in verified:
        verified = {"chain_id":verified["chain_id"],"finalized_slot":verified["finalized_slot"],
                    "execution_height":int(verified["header"]["number"],16)}
    print(json.dumps({"verified": verified, "download_bytes": transport.total,
                      "requests": transport.calls, "evidence": str(args.out)}))


if __name__ == "__main__":
    main()
