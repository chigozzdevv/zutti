import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def group_processes(group):
    output = subprocess.run(
        ["ps", "-axo", "pid=,pgid=,rss="],
        check=True, capture_output=True, text=True, timeout=10,
    ).stdout
    processes = {}
    for line in output.splitlines():
        parts = line.split()
        if len(parts) == 3 and int(parts[1]) == group:
            processes[int(parts[0])] = int(parts[2])
    return processes


def stop(process):
    processes = group_processes(process.pid)
    for pid in sorted(processes, reverse=True):
        if pid <= 1 or pid == os.getpid():
            raise RuntimeError("refusing to signal an invalid child process")
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        process.poll()
        if not group_processes(process.pid):
            break
        time.sleep(0.05)
    else:
        for pid in group_processes(process.pid):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description="Bound a local SPV build or proof process group")
    parser.add_argument("--seconds", type=int, default=600)
    parser.add_argument("--rss-mib", type=int, default=4096)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not 1 <= args.seconds <= 3600 or not 128 <= args.rss_mib <= 8192:
        parser.error("invalid command or resource bounds")
    report_path = args.log.with_suffix(args.log.suffix + ".json")
    if args.log.exists() or report_path.exists():
        parser.error("output already exists")
    environment = os.environ.copy()
    environment.pop("RISC0_DEV_MODE", None)
    environment["RAYON_NUM_THREADS"] = "2"
    environment["OMP_NUM_THREADS"] = "2"
    environment["HTTP_PROXY"] = "http://127.0.0.1:9"
    environment["HTTPS_PROXY"] = "http://127.0.0.1:9"
    environment["ALL_PROXY"] = "http://127.0.0.1:9"
    environment["NO_PROXY"] = "127.0.0.1,localhost,::1"
    descriptor = os.open(args.log, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    started = time.monotonic()
    peak = 0
    status = "failed"
    failure = None
    with os.fdopen(descriptor, "wb") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                   env=environment, start_new_session=True)
        try:
            while process.poll() is None:
                peak = max(peak, sum(group_processes(process.pid).values()))
                if peak > args.rss_mib * 1024:
                    status = "memory_limit"
                    stop(process)
                    break
                if time.monotonic() - started >= args.seconds:
                    status = "timeout"
                    stop(process)
                    break
                time.sleep(0.2)
            else:
                status = "completed" if process.returncode == 0 else "failed"
        except (Exception, KeyboardInterrupt) as error:
            status = "monitor_error"
            failure = str(error)
        finally:
            try:
                stop(process)
            except Exception as error:
                status = "cleanup_error"
                failure = str(error)
    report = {"status": status, "exit_code": process.returncode,
              "seconds": time.monotonic() - started, "sampled_peak_rss_mib": peak / 1024,
              "seconds_limit": args.seconds, "rss_limit_mib": args.rss_mib,
              "development_mode": False, "paid_jobs": 0, "error": failure}
    descriptor = os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps(report))
    return 0 if status == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
