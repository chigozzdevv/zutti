import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shlex
import signal
import struct
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit, urlunsplit

from source_rpc import MAX_RESPONSE, private_bytes

COUNT_FIELDS = (
    "transactions", "non_coinbase_transactions", "transparent_inputs", "transparent_outputs",
    "sprout_phgr_proofs", "sprout_groth16_proofs", "sprout_signatures", "sapling_bundles",
    "sapling_spends", "sapling_outputs", "orchard_bundles", "orchard_actions",
    "ironwood_bundles", "ironwood_actions",
)
ROW_FIELDS = {
    "schema", "height", "block_hash", "parent_hash", "branch", "branch_id", "block_bytes",
    "counts", "versions", "consensus_verified", "proof_generated",
}
BRANCHES = {"Sprout", "Overwinter", "Sapling", "Blossom", "Heartwood", "Canopy", "Nu5", "Nu6", "Nu6_1", "Nu6_2", "Nu6_3", "Nu7"}
HASH = re.compile(r"[0-9a-f]{64}\Z")
RUN = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}\Z")


def digest_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(65_536):
            digest.update(chunk)
    return digest.hexdigest()


def read_line(child, limit, timeout):
    deadline = time.monotonic() + timeout
    data = bytearray()
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([child.stdout], [], [], remaining)[0]:
            raise TimeoutError("inventory child exceeded its response deadline")
        chunk = os.read(child.stdout.fileno(), min(65_536, limit + 1 - len(data)))
        if not chunk:
            raise RuntimeError("inventory child closed an incomplete frame")
        data.extend(chunk)
        if len(data) > limit:
            raise RuntimeError("inventory child exceeded its frame bound")
        if data.endswith(b"\n"):
            return json.loads(data)


def write_all(stream, data):
    remaining = memoryview(data)
    while remaining:
        written = stream.write(remaining)
        if not written:
            raise RuntimeError("inventory child stopped accepting input")
        remaining = remaining[written:]


def stop(child):
    if child is None:
        return
    if child.stdin and not child.stdin.closed:
        try:
            child.stdin.close()
        except BrokenPipeError:
            pass
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
    if child.stdout:
        child.stdout.close()


class RpcProxy:
    def __init__(self, endpoint, cookie, container):
        prefix = []
        if container:
            if not RUN.fullmatch(container):
                raise ValueError("invalid source container name")
            pid = subprocess.check_output(
                ["docker", "inspect", "--format", "{{.State.Pid}}", container], text=True, timeout=10,
            ).strip()
            if not pid.isdecimal() or int(pid) == 0:
                raise ValueError("source container is not running")
            prefix = ["nsenter", "-t", pid, "-n"]
        self.child = subprocess.Popen(
            prefix + [sys.executable, str(Path(__file__).with_name("source_rpc.py")),
                      "--endpoint", endpoint, "--cookie", str(cookie)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0,
        )

    def call(self, method, params):
        request = json.dumps({"method": method, "params": params}).encode() + b"\n"
        write_all(self.child.stdin, request)
        response = read_line(self.child, MAX_RESPONSE + 128, 35)
        if response.get("ok") is not True:
            raise RuntimeError(f"source RPC failed for {method}")
        return response["result"]

    def close(self):
        stop(self.child)


def validate_row(row, height, parent):
    if not isinstance(row, dict) or set(row) != ROW_FIELDS:
        raise ValueError("invalid inventory row fields")
    if type(row["schema"]) is not int or row["schema"] != 1 or type(row["height"]) is not int or row["height"] != height:
        raise ValueError("inventory height mismatch")
    if not HASH.fullmatch(row["block_hash"]) or row["parent_hash"] != parent:
        raise ValueError("inventory block ancestry mismatch")
    if row["branch"] not in BRANCHES or not re.fullmatch(r"[0-9a-f]{8}", row["branch_id"]):
        raise ValueError("unsupported inventory branch")
    if type(row["block_bytes"]) is not int or not 0 < row["block_bytes"] <= 2_000_000:
        raise ValueError("inventory block size mismatch")
    if row["consensus_verified"] is not False or row["proof_generated"] is not False:
        raise ValueError("inventory cannot carry settlement authority")
    counts = row["counts"]
    if not isinstance(counts, dict) or set(counts) != set(COUNT_FIELDS):
        raise ValueError("invalid inventory count fields")
    if any(type(v) is not int or not 0 <= v <= 2_000_000 for v in counts.values()):
        raise ValueError("invalid inventory operation count")
    if counts["transactions"] < 1 or counts["non_coinbase_transactions"] != counts["transactions"] - 1:
        raise ValueError("inventory transaction count mismatch")
    versions = row["versions"]
    if not isinstance(versions, dict) or not versions or any(
        k not in {"1", "2", "3", "4", "5", "6"} or type(v) is not int or not 0 < v <= 100_000
        for k, v in versions.items()
    ) or sum(versions.values()) != counts["transactions"]:
        raise ValueError("inventory transaction-version count mismatch")


def group_key(row):
    counts = row["counts"]
    pools = [name for name, fields in (
        ("phgr", ("sprout_phgr_proofs",)), ("sprout", ("sprout_groth16_proofs",)),
        ("sapling", ("sapling_spends", "sapling_outputs")),
        ("orchard", ("orchard_actions",)), ("ironwood", ("ironwood_actions",)),
    ) if any(counts[field] for field in fields)]
    operations = sum(counts[k] for k in (
        "transparent_inputs", "sprout_phgr_proofs", "sprout_groth16_proofs", "sapling_spends",
        "sapling_outputs", "orchard_actions", "ironwood_actions",
    ))
    band = min(7, (max(1, operations).bit_length() - 1) // 2)
    return row["branch"] + "_" + ("-".join(pools) or "transparent") + "_" + str(band)


def add_row(totals, groups, row):
    totals["blocks"] += 1
    totals["block_bytes"] += row["block_bytes"]
    for field, value in row["counts"].items():
        totals[field] += value
    key = group_key(row)
    group = groups.setdefault(key, {
        "blocks": 0, "block_bytes": 0, "counts": {key: 0 for key in COUNT_FIELDS},
        "samples": [], "largest": [], "first_height": row["height"], "last_height": row["height"],
    })
    group["blocks"] += 1
    group["block_bytes"] += row["block_bytes"]
    group["last_height"] = row["height"]
    for field, value in row["counts"].items():
        group["counts"][field] += value
    candidate = {"height": row["height"], "hash": row["block_hash"], "bytes": row["block_bytes"]}
    sample = dict(candidate, priority=hashlib.sha256((key + row["block_hash"]).encode()).hexdigest())
    group["samples"] = sorted(group["samples"] + [sample], key=lambda item: item["priority"])[:3]
    group["largest"] = sorted(group["largest"] + [candidate], key=lambda item: (-item["bytes"], item["height"]))[:3]


class InventoryStore:
    def __init__(self, config, prefix="source_inventory_"):
        from pymongo import MongoClient
        from pymongo.write_concern import WriteConcern
        if set(config) != {"uri", "database"} or not RUN.fullmatch(config["database"]):
            raise ValueError("invalid MongoDB inventory configuration")
        self.client = MongoClient(config["uri"], appname="zutti-source-inventory", maxPoolSize=2,
                                  serverSelectionTimeoutMS=5000, socketTimeoutMS=15000)
        database = self.client.get_database(config["database"], write_concern=WriteConcern(w=1, j=True))
        self.runs = database[prefix + "runs"]
        self.blocks = database[prefix + "blocks"]
        self.blocks.create_index([("run_id", 1), ("height", 1)], unique=True)
        self.state = None

    def open(self, run_id, binding, parent):
        initial = {
            "_id": run_id, "binding": binding, "next_height": binding["start_height"],
            "last_hash": parent, "totals": dict.fromkeys(("blocks", "block_bytes") + COUNT_FIELDS, 0),
            "groups": {}, "scan_seconds": 0.0,
        }
        self.runs.update_one({"_id": run_id}, {"$setOnInsert": initial}, upsert=True)
        state = self.runs.find_one({"_id": run_id})
        if state["binding"] != binding:
            raise ValueError("inventory run binding differs; use a new run identifier")
        if not binding["start_height"] <= state["next_height"] <= binding["target_height"] + 1:
            raise ValueError("invalid inventory frontier")
        if state["totals"]["blocks"] != state["next_height"] - binding["start_height"]:
            raise ValueError("inventory frontier count mismatch")
        if state["next_height"] > binding["start_height"]:
            last = self.blocks.find_one({"run_id": run_id, "height": state["next_height"] - 1})
            if last is None or last["block_hash"] != state["last_hash"]:
                raise ValueError("inventory frontier row is unavailable")
        elif state["last_hash"] != parent:
            raise ValueError("inventory starting parent differs")
        self.state = state
        return copy.deepcopy(state)

    def commit(self, rows, seconds):
        from pymongo import UpdateOne
        if not rows or len(rows) > 512 or seconds < 0:
            raise ValueError("invalid inventory commit")
        state = copy.deepcopy(self.state)
        operations = []
        for row in rows:
            validate_row(row, state["next_height"], state["last_hash"])
            if row["height"] > state["binding"]["target_height"]:
                raise ValueError("inventory commit exceeds its pinned target")
            document = dict(row, _id=state["_id"] + ":" + str(row["height"]), run_id=state["_id"])
            operations.append(UpdateOne(document, {"$setOnInsert": document}, upsert=True))
            state["next_height"] += 1
            state["last_hash"] = row["block_hash"]
            add_row(state["totals"], state["groups"], row)
        state["scan_seconds"] += seconds
        self.blocks.bulk_write(operations, ordered=True)
        updated = self.runs.replace_one({"_id": state["_id"], "next_height": self.state["next_height"],
                                         "last_hash": self.state["last_hash"]}, state)
        if updated.modified_count != 1:
            raise RuntimeError("inventory frontier changed concurrently")
        self.state = state

    def close(self):
        self.client.close()


def report(state, processed, elapsed):
    remaining = state["binding"]["target_height"] + 1 - state["next_height"]
    rate = processed / elapsed if processed and elapsed > 0 else None
    groups = sorted(state["groups"].items())
    value = {
        "schema": 1, "run_id": state["_id"], "binding": state["binding"],
        "next_height": state["next_height"], "last_hash": state["last_hash"],
        "complete": remaining == 0, "remaining_blocks": remaining, "totals": state["totals"],
        "session_blocks": processed, "session_seconds": elapsed, "session_blocks_per_second": rate,
        "scan_eta_seconds_at_session_rate": remaining / rate if rate else None,
        "groups_count": len(groups), "groups_truncated": len(groups) > 128,
        "groups": dict(groups[:128]), "total_cycle_estimate": None,
        "consensus_verified": False, "proof_generated": False, "paid_jobs": 0,
    }
    encoded = json.dumps(value, indent=2).encode() + b"\n"
    if len(encoded) > 1_000_000:
        raise ValueError("inventory summary exceeds its export bound")
    return value, encoded


def save_report(path, encoded):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as out:
        name = out.name
        out.write(encoded)
        out.flush()
        os.fsync(out.fileno())
    try:
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def mongodb_config(path, container=None):
    data = private_bytes(path, 65_536)
    if data.lstrip().startswith(b"{"):
        config = json.loads(data)
    else:
        fields = {}
        for line in data.decode().splitlines():
            name, separator, value = line.strip().partition("=")
            if name in {"ZUTTI_MONGODB_URI", "ZUTTI_MONGODB_DATABASE"}:
                values = shlex.split(value)
                if not separator or len(values) != 1 or name in fields:
                    raise ValueError("invalid MongoDB storage configuration")
                fields[name] = values[0]
        config = {"uri": fields["ZUTTI_MONGODB_URI"], "database": fields["ZUTTI_MONGODB_DATABASE"]}
    if container:
        if not RUN.fullmatch(container):
            raise ValueError("invalid MongoDB container name")
        uri = urlsplit(config["uri"])
        if uri.scheme != "mongodb" or "," in uri.netloc:
            raise ValueError("container resolution requires a single MongoDB endpoint")
        info = json.loads(subprocess.check_output(["docker", "inspect", container], text=True, timeout=10))[0]
        if not info["State"]["Running"]:
            raise ValueError("MongoDB container is not running")
        addresses = {network["IPAddress"] for network in info["NetworkSettings"]["Networks"].values()
                     if network["IPAddress"] and uri.hostname in (network.get("Aliases") or []) + (network.get("DNSNames") or [])}
        if len(addresses) != 1:
            raise ValueError("MongoDB host does not identify the selected container")
        authentication = uri.netloc.rsplit("@", 1)[0] + "@" if "@" in uri.netloc else ""
        authority = authentication + addresses.pop() + ":" + str(uri.port or 27017)
        config["uri"] = urlunsplit((uri.scheme, authority, uri.path, uri.query, uri.fragment))
    return config


def arguments():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc", required=True)
    parser.add_argument("--rpc-cookie", required=True, type=Path)
    parser.add_argument("--node-container")
    parser.add_argument("--scanner", required=True, type=Path)
    parser.add_argument("--mongodb-config", required=True, type=Path)
    parser.add_argument("--mongodb-container")
    parser.add_argument("--network", choices=["testnet", "mainnet"], required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--start-height", type=int, default=0)
    parser.add_argument("--through-height", type=int, required=True)
    parser.add_argument("--max-blocks", type=int, default=2000)
    parser.add_argument("--max-seconds", type=int, default=300)
    parser.add_argument("--max-rate", type=float, default=100)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not RUN.fullmatch(args.run) or not 0 <= args.start_height <= args.through_height < 500_000_000:
        parser.error("invalid run identifier or source-height interval")
    if not 0 < args.max_blocks <= 10_000_000 or not 0 < args.max_seconds <= 86_400:
        parser.error("invalid inventory execution bounds")
    if not 0 < args.max_rate <= 1000 or not 0 < args.batch_size <= 512:
        parser.error("invalid inventory rate or batch size")
    return args


def run(args):
    os.umask(0o077)
    rpc = RpcProxy(args.rpc, args.rpc_cookie, args.node_container)
    store = None
    child = None
    try:
        info = rpc.call("getblockchaininfo", [])
        if info["chain"] != ("test" if args.network == "testnet" else "main"):
            raise ValueError("source network differs from the inventory profile")
        if args.through_height > info["blocks"]:
            raise ValueError("inventory target exceeds downloaded source history")
        binding = {
            "schema": 1, "network": args.network, "start_height": args.start_height,
            "target_height": args.through_height,
            "target_hash": rpc.call("getblockhash", [args.through_height]),
            "scanner_sha256": digest_file(args.scanner),
        }
        parent = rpc.call("getblockhash", [args.start_height - 1]) if args.start_height else "0" * 64
        store = InventoryStore(mongodb_config(args.mongodb_config, args.mongodb_container))
        state = store.open(args.run, binding, parent)
        height = state["next_height"]
        if height and rpc.call("getblockhash", [height - 1]) != state["last_hash"]:
            raise ValueError("retained inventory frontier is on a different node branch")
        count = min(args.max_blocks, binding["target_height"] + 1 - height)
        started = time.monotonic()
        if count:
            child = subprocess.Popen(
                [str(args.scanner), "--network", args.network, "--start-height", str(height),
                 "--parent", state["last_hash"], "--max-blocks", str(count)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0,
            )
        processed = 0
        rows = []
        committed = started
        parent = state["last_hash"]
        for current in range(height, height + count):
            if time.monotonic() - started >= args.max_seconds:
                break
            raw = bytes.fromhex(rpc.call("getblock", [str(current), 0]))
            if not 0 < len(raw) <= 2_000_000:
                raise ValueError("invalid serialized source block size")
            write_all(child.stdin, struct.pack("<I", len(raw)))
            write_all(child.stdin, raw)
            row = read_line(child, 16_384, 30)
            validate_row(row, current, parent)
            parent = row["block_hash"]
            rows.append(row)
            processed += 1
            if len(rows) == args.batch_size:
                if rpc.call("getblockhash", [current]) != parent or rpc.call("getblockhash", [args.through_height]) != binding["target_hash"]:
                    raise ValueError("source branch changed during the inventory scan")
                store.commit(rows, time.monotonic() - committed)
                rows = []
                committed = time.monotonic()
                value, encoded = report(store.state, processed, time.monotonic() - started)
                save_report(args.out, encoded)
                print(json.dumps({k: value[k] for k in ("run_id", "next_height", "remaining_blocks", "session_blocks_per_second")}), flush=True)
            delay = processed / args.max_rate - (time.monotonic() - started)
            if delay > 0:
                time.sleep(min(delay, max(0.0, args.max_seconds - (time.monotonic() - started))))
        if rows:
            if rpc.call("getblockhash", [rows[-1]["height"]]) != rows[-1]["block_hash"] or rpc.call("getblockhash", [args.through_height]) != binding["target_hash"]:
                raise ValueError("source branch changed before the inventory commit")
            store.commit(rows, time.monotonic() - committed)
        if child:
            child.stdin.close()
            child.wait(timeout=5)
            if child.returncode != 0:
                raise RuntimeError("inventory scanner did not complete successfully")
        value, encoded = report(store.state, processed, time.monotonic() - started)
        save_report(args.out, encoded)
        print(json.dumps({k: v for k, v in value.items() if k != "groups"}), flush=True)
    finally:
        stop(child)
        rpc.close()
        if store:
            store.close()


def interrupted(*_):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        run(arguments())
    except KeyboardInterrupt:
        print("inventory interrupted; committed progress is retained", file=sys.stderr)
        sys.exit(130)
    except Exception as error:
        detail = str(error) if isinstance(error, (ValueError, RuntimeError, TimeoutError)) else type(error).__name__
        print(f"inventory stopped: {detail}", file=sys.stderr)
        sys.exit(1)
