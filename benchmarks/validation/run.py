import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time


ROOT = Path(__file__).resolve().parent
CASES = {}
for kind, height, pool, actions in (
    ("orchard-original-2", 1886667, "orchard", 2),
    ("orchard-original-6", 1986500, "orchard", 6),
    ("orchard-fixed-1", 4119011, "orchard", 1),
    ("orchard-fixed-2", 4094265, "orchard", 2),
    ("ironwood-1", 4469047, "ironwood", 1),
    ("ironwood-2", 4520552, "ironwood", 2),
):
    for repetitions in (1, 2):
        CASES[f"{kind}-r{repetitions}"] = {
            "height": height, "pool": pool, "actions": actions, "repetitions": repetitions,
        }
for kind, height, pool in (
    ("orchard-1", 4119011, "orchard"),
    ("orchard-4", 4094265, "orchard"),
    ("ironwood-1", 4469047, "ironwood"),
    ("ironwood-7", 4520552, "ironwood"),
):
    for repetitions in (1, 16):
        CASES[f"trees-{kind}-r{repetitions}"] = {
            "height": height, "pool": pool, "repetitions": repetitions, "trees": True,
        }
for kind, height, pool, actions in (
    ("orchard-original", 1886667, "orchard", 2),
    ("orchard-fixed", 4119011, "orchard", 1),
    ("ironwood", 4469047, "ironwood", 1),
):
    for tamper in ("binding", "proof"):
        CASES[f"{kind}-changed-{tamper}"] = {
            "height": height, "pool": pool, "actions": actions,
            "repetitions": 1, "tamper": tamper,
        }
for kind, height, pool, actions in (
    ("orchard-original-2", 1886667, "orchard", 2),
    ("orchard-fixed-1", 4119011, "orchard", 1),
    ("ironwood-1", 4469047, "ironwood", 1),
):
    for repetitions in (1, 16):
        CASES[f"context-{kind}-r{repetitions}"] = {
            "height": height, "pool": pool, "actions": actions,
            "repetitions": repetitions, "context": True,
        }


def fixture(height):
    manifest = json.loads((ROOT / "fixtures/manifest.json").read_text())
    entries = [row for row in manifest["blocks"] if row["height"] == height]
    if manifest["network"] != "testnet" or len(entries) != 1:
        raise ValueError("fixture manifest does not identify one Testnet block")
    entry = entries[0]
    path = ROOT / "fixtures" / entry["filename"]
    data = path.read_bytes()
    if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise ValueError("public block fixture digest mismatch")
    return path


def terminate(child):
    if child.poll() is None:
        os.killpg(child.pid, signal.SIGTERM)
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()


def run_case(args, name, program):
    case = CASES[name]
    out = args.out / name
    log = args.out / f"{name}.log"
    command = [str(args.runner.resolve()), "--network", "testnet", "--block",
               str(fixture(case["height"])), "--pool", case["pool"], "--repetitions",
               str(case["repetitions"]), "--cycle-limit", str(args.cycle_limit), "--out", str(out)]
    for option in ("actions", "tamper"):
        if option in case:
            command += ["--" + option, str(case[option])]
    if case.get("trees"):
        command.append("--trees")
    if case.get("context"):
        command.append("--context")
    if program:
        command += ["--program", str(program.resolve())]
    started = time.monotonic()
    with log.open("x") as output:
        child = subprocess.Popen(
            ["nice", "-n", "10"] + command, stdout=output, stderr=subprocess.STDOUT,
            start_new_session=True, env=dict(os.environ, RISC0_DEV_MODE="0"),
        )
        timed_out = False
        try:
            child.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
        finally:
            terminate(child)
    report = {"case": name, "returncode": child.returncode, "timed_out": timed_out,
              "wall_seconds": time.monotonic() - started, "cycle_limit": args.cycle_limit,
              "proof_generated": False, "paid_jobs": 0}
    result = out / ("rejection.json" if "tamper" in case else "execution.json")
    if child.returncode == 0 and not timed_out:
        report["measurement"] = json.loads(result.read_text())
    else:
        match = re.search(r"Session limit exceeded: (\d+) >= (\d+)", log.read_text())
        if match and int(match[2]) == args.cycle_limit:
            report["cycles_lower_bound"] = args.cycle_limit
        report["completed_measurement"] = False
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--case", choices=sorted(CASES), action="append", required=True)
    parser.add_argument("--program", type=Path)
    parser.add_argument("--tree-program", type=Path)
    parser.add_argument("--context-program", type=Path)
    parser.add_argument("--cycle-limit", type=int, default=1 << 38)
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args()
    if not 0 < args.cycle_limit <= 1 << 38 or not 10 <= args.timeout <= 7200:
        parser.error("execution bounds are outside the supported range")
    if len(set(args.case)) != len(args.case):
        parser.error("measurement cases must be unique")
    args.out.mkdir(parents=True, exist_ok=False)
    programs = {"authorization": args.program, "trees": args.tree_program, "context": args.context_program}
    reports = []
    for name in args.case:
        kind = "trees" if CASES[name].get("trees") else "context" if CASES[name].get("context") else "authorization"
        report = run_case(args, name, programs[kind])
        reports.append(report)
        (args.out / "measurements.json").write_text(json.dumps(reports, indent=2) + "\n")
        print(json.dumps(report), flush=True)
        if report["returncode"] or report["timed_out"]:
            raise SystemExit(1)
        programs[kind] = args.out / name / "program.elf"


if __name__ == "__main__":
    main()
