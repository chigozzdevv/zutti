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


def reconcile(args):
    with args.ledger.with_suffix(".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        ledger = private_json(args.ledger)
        records = [record for record in ledger["requests"] if record["name"] == args.settle]
        if len(records) != 1:
            raise ValueError("One existing proof request is required")
        record = records[0]
        if record.get("state") == "fulfilled":
            return
        if record.get("state") != "submitted" or not record.get("request_id"):
            raise ValueError("Unknown submission outcomes must remain reserved")
        evidence = args.ledger.parent / f"{record['name']}-market-evidence.json"
        if not evidence.exists():
            subprocess.run([str(args.binary), "--network", "base", "--config", str(args.config),
                "evidence", "--request", record["request"], "--out", str(evidence)],
                check=True, timeout=240, capture_output=True)
        value = private_json(evidence)
        ceiling = int(record["reserved_wei"])
        price = int(value["price_wei"])
        issued = private_json(Path(record["request"]))
        if (value["chain_id"] != 8453 or value["request_id"] != record["request_id"]
                or value["image_id"] != issued["image_id"] or not value.get("fulfillment_transaction")
                or not value.get("fulfillment_block") or not 0 <= price <= ceiling):
            raise ValueError("Fulfillment fee evidence does not match the reserved request")
        record.update({"state":"fulfilled", "ceiling_wei":str(ceiling), "spent_wei":str(price),
            "reserved_wei":str(price), "fee_evidence":str(evidence), "reconciled_at":int(time.time())})
        save(args.ledger, ledger)
        print(json.dumps({"name":record["name"], "spent_wei":str(price),
            "released_wei":str(ceiling-price), "requests_attempted":len(ledger["requests"])}))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--job", type=Path)
    parser.add_argument("--name")
    parser.add_argument("--settle")
    args = parser.parse_args()
    if args.settle:
        if args.job or args.name:
            parser.error("Fee reconciliation cannot submit another proof")
        reconcile(args)
    else:
        if not args.job or not args.name:
            parser.error("Submission requires --job and --name")
        submit(args)


if __name__ == "__main__":
    main()
