import argparse
import fcntl
import hashlib
import json
import os
import re
import subprocess
import tempfile
import time
from pathlib import Path


def save(path, value):
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as output:
        json.dump(value, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
        temporary = output.name
    os.replace(temporary, path)
    parent = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(parent)
    finally:
        os.close(parent)


def private_json(path):
    if path.is_symlink() or path.stat().st_mode & 0o077:
        raise ValueError("Proof records must be private regular files")
    if not path.is_file() or path.stat().st_size > 150_000:
        raise ValueError("Invalid proof record")
    return json.loads(path.read_text())


def submit(args):
    if not re.fullmatch(r"[a-z][a-z0-9-]{0,63}", args.name):
        raise ValueError("Invalid proof operation name")
    if args.config.is_symlink() or args.config.stat().st_mode & 0o077:
        raise ValueError("Prover configuration must be private")
    config = args.config.read_text()
    matches = re.findall(r"^BOUNDLESS_MAX_PRICE_WEI=([0-9]+)$", config, re.MULTILINE)
    if len(matches) != 1:
        raise ValueError("A single explicit prover price cap is required")
    cap = int(matches[0])
    job = private_json(args.job)
    if job["visibility"] != "public":
        raise ValueError("This approval covers public Testnet inputs only")
    identity = hashlib.sha256(json.dumps({key: job[key] for key in
        ("image_id", "stdin_sha256", "journal_sha256", "cycles")}, sort_keys=True).encode()).hexdigest()
    with args.ledger.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        ledger = private_json(args.ledger)
        if (ledger["network"] != "base" or ledger["automatic_resubmission"] is not False
                or ledger["testnet_witnesses_public"] is not True):
            raise ValueError("Approval does not cover this run")
        records = ledger["requests"]
        duplicate = False
        for record in records:
            previous = private_json(Path(record["job"]))
            duplicate |= previous["image_id"] == job["image_id"] and previous["journal_sha256"] == job["journal_sha256"]
        if duplicate or any(record["name"] == args.name or record["job_identity"] == identity for record in records):
            raise ValueError("This proof was already attempted; inspect or collect its existing request")
        reserved = sum(int(record["reserved_wei"]) for record in records)
        if (cap <= 0 or cap > int(ledger["per_request_cap_wei"])
                or reserved + cap > int(ledger["total_cap_wei"])
                or len(records) >= ledger["maximum_requests"]):
            raise ValueError("Proof exceeds the approved run budget")
        request = args.ledger.parent / f"{args.name}-request.json"
        log = args.ledger.parent / f"{args.name}-submit.log"
        if request.exists() or log.exists():
            raise ValueError("Proof artifacts already exist")
        record = {"name": args.name, "job_identity": identity, "job": str(args.job.resolve()),
                  "request": str(request), "reserved_wei": str(cap), "state": "attempting",
                  "created_at": int(time.time()), "automatic_resubmission": False}
        records.append(record)
        save(args.ledger, ledger)
        descriptor = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            try:
                result = subprocess.run([str(args.binary), "--network", "base", "--config",
                    str(args.config), "submit", "--job", str(args.job), "--request", str(request)],
                    stdout=output, stderr=subprocess.STDOUT, timeout=240, check=False)
                record["exit_code"] = result.returncode
            except subprocess.TimeoutExpired:
                record["state"] = "unknown"
            if request.exists():
                issued = private_json(request)
                if int(issued["max_price_wei"]) > cap or issued["chain_id"] != 8453:
                    raise ValueError("Issued request does not match its approval")
                record["request_id"] = issued["request_id"]
                record["expires_at"] = issued["expires_at"]
            if record["state"] != "unknown":
                record["state"] = "submitted" if record["exit_code"] == 0 else "inspect_required"
        save(args.ledger, ledger)
        print(json.dumps({"name": args.name, "state": record["state"],
            "request_id": record.get("request_id"), "requests_attempted": len(records),
            "reserved_wei": str(reserved + cap), "total_cap_wei": ledger["total_cap_wei"]}))
        if record["state"] != "submitted":
            raise RuntimeError("Inspect the retained private log and request; automatic resubmission is disabled")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--job", type=Path, required=True)
    parser.add_argument("--name", required=True)
    submit(parser.parse_args())


if __name__ == "__main__":
    main()
