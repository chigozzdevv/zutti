import argparse
import base64
import http.client
import json
import os
from pathlib import Path
import stat
import sys
from urllib.parse import urlsplit

MAX_RESPONSE = 4_001_024
METHODS = {"getblockchaininfo", "getblock", "getblockhash"}


def private_json(path):
    return json.loads(private_bytes(path, 16_384))


def private_bytes(path, limit):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source:
        meta = os.fstat(source.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_mode & 0o077 or meta.st_uid != os.geteuid():
            raise ValueError("configuration must be an owner-only regular file")
        value = source.read(limit + 1)
    if len(value) > limit:
        raise ValueError("configuration exceeds its size bound")
    return value


class Rpc:
    def __init__(self, endpoint, cookie):
        url = urlsplit(endpoint)
        if url.scheme != "http" or url.hostname not in {"localhost", "127.0.0.1", "::1"}:
            raise ValueError("source RPC must use a loopback HTTP endpoint")
        if url.username or url.password or url.query or url.fragment:
            raise ValueError("invalid source RPC URL")
        value = private_bytes(cookie, 4096).strip()
        if b":" not in value or b"\n" in value or b"\r" in value:
            raise ValueError("invalid RPC cookie")
        self.authorization = "Basic " + base64.b64encode(value).decode()
        self.connection = http.client.HTTPConnection(url.hostname, url.port, timeout=30)
        self.path = url.path or "/"
        self.identifier = 0

    def call(self, method, params):
        if method not in METHODS or not isinstance(params, list):
            raise ValueError("unsupported read-only source RPC method")
        self.identifier += 1
        payload = json.dumps({"jsonrpc": "2.0", "id": self.identifier, "method": method, "params": params})
        self.connection.request("POST", self.path, body=payload,
                                headers={"Content-Type": "application/json", "Authorization": self.authorization})
        response = self.connection.getresponse()
        data = response.read(MAX_RESPONSE + 1)
        if response.status != 200 or len(data) > MAX_RESPONSE:
            raise RuntimeError("source RPC transport failed or exceeded its response bound")
        value = json.loads(data)
        if value.get("id") != self.identifier or value.get("error") or value.get("result") is None:
            raise RuntimeError(f"source RPC rejected {method}")
        return value["result"]

    def close(self):
        self.connection.close()


def agent():
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--cookie", required=True, type=Path)
    args = parser.parse_args()
    rpc = Rpc(args.endpoint, args.cookie)
    try:
        while line := sys.stdin.buffer.readline(4097):
            if len(line) > 4096 or not line.endswith(b"\n"):
                raise ValueError("oversized RPC agent request")
            request = json.loads(line)
            try:
                result = {"ok": True, "result": rpc.call(request["method"], request["params"])}
            except Exception:
                result = {"ok": False, "error": "source RPC request failed"}
            print(json.dumps(result, separators=(",", ":")), flush=True)
    finally:
        rpc.close()


if __name__ == "__main__":
    agent()
