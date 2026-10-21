import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import signal
import struct
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stop(child):
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        child.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()


def measured(command, env, log, timeout, memory_limit):
    start = time.monotonic()
    peak = 0
    stopped = None
    with log.open("w") as stream:
        child = subprocess.Popen(
            ["/usr/bin/time", "-l", *command],
            env=env, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            while child.poll() is None:
                if time.monotonic() - start > timeout:
                    stopped = "wall_time_limit"
                    break
                snapshot = subprocess.run(
                    ["ps", "-axo", "pgid=,rss="], capture_output=True, text=True, timeout=3,
                )
                if snapshot.returncode:
                    stopped = "memory_monitor_unavailable"
                    break
                rss = sum(
                    int(parts[1]) * 1024
                    for line in snapshot.stdout.splitlines()
                    if len(parts := line.split()) == 2 and int(parts[0]) == child.pid
                )
                peak = max(peak, rss)
                if rss > memory_limit:
                    stopped = "memory_limit"
                    break
                time.sleep(0.1)
        finally:
            if child.poll() is None:
                stop(child)
            child.wait()
    text = log.read_text()
    maximum = re.search(r"(\d+)\s+maximum resident set size", text)
    cpu = re.search(r"([\d.]+)\s+real\s+([\d.]+)\s+user\s+([\d.]+)\s+sys", text)
    return {
        "exit_code": child.returncode,
        "wall_seconds": time.monotonic() - start,
        "sampled_peak_group_rss_bytes": peak,
        "maximum_child_rss_bytes": int(maximum[1]) if maximum else None,
        "user_cpu_seconds": float(cpu[2]) if cpu else None,
        "system_cpu_seconds": float(cpu[3]) if cpu else None,
        "stopped_for": stopped,
        "timeout_seconds": timeout,
        "memory_limit_bytes": memory_limit,
    }


def plonk_key(source):
    text = (source / "zisk-contracts/PlonkVerifier.sol").read_text()
    constants = {
        name: int(value)
        for name, value in re.findall(r"uint(?:16|32|256)\s+constant\s+(\w+)\s*=\s*(\d+)\s*;", text)
    }
    if constants["n"] != 1 << 24:
        raise RuntimeError("Unexpected PLONK domain")
    key = {
        "protocol": "plonk", "curve": "bn128", "nPublic": constants["nPublic"],
        "power": 24, "k1": str(constants["k1"]), "k2": str(constants["k2"]),
        "w": str(constants["w1"]),
    }
    for name in ["Qm", "Ql", "Qr", "Qo", "Qc", "S1", "S2", "S3"]:
        key[name] = [str(constants[name + "x"]), str(constants[name + "y"]), "1"]
    key["X_2"] = [
        [str(constants["X2x1"]), str(constants["X2x2"])],
        [str(constants["X2y1"]), str(constants["X2y2"])], ["1", "0"],
    ]
    return key


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--toolchain-root", required=True, type=Path)
    parser.add_argument("--setup-key", required=True, type=Path)
    parser.add_argument("--solc", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise RuntimeError("This resource monitor uses macOS time and RSS units")
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    trusted = json.loads((HERE / "verification-fixture.json").read_text())
    root = args.toolchain_root.resolve()
    versions = json.loads((root / "versions.json").read_text())
    if versions["zisk"] != trusted["zisk"] or versions["zisk_commit"] != trusted["commit"]:
        raise RuntimeError("Unexpected toolchain profile")
    source = root / "source" / ("zisk-" + trusted["commit"])
    for name, expected in trusted["source_sha256"].items():
        if digest(source / name) != expected:
            raise RuntimeError(f"Changed upstream source: {name}")
    if digest(args.setup_key) != trusted["setup_key_sha256"]:
        raise RuntimeError("Unrecognized trusted setup key")
    setup = list(struct.unpack("<4Q", args.setup_key.read_bytes()))
    expected_root = "0x" + b"".join(v.to_bytes(8, "big") for v in setup).hex()
    if expected_root != trusted["rootCVadcopFinal"]:
        raise RuntimeError("Setup root does not match the pinned fixture")
    if not args.solc.is_file() or not os.access(args.solc, os.X_OK):
        raise RuntimeError("A cached executable Solidity compiler is required")
    forge = shutil.which("forge")
    if not forge:
        raise RuntimeError("A cached Forge executable is required")
    work = args.work_dir.resolve()
    work.mkdir(mode=0o700, parents=True, exist_ok=False)
    env = os.environ.copy()
    env.update({
        "DYLD_LIBRARY_PATH": ":".join(json.loads((root / "runtime-paths.json").read_text())),
        "ZISK_HOME": str(root), "OMP_NUM_THREADS": "2", "RAYON_NUM_THREADS": "2",
    })
    (work / "trusted-setup.json").write_text(json.dumps(setup) + "\n")
    (work / "trusted-plonk.json").write_text(json.dumps(plonk_key(source), indent=2) + "\n")
    proof = source / "zisk-contracts/test/plonk_proof.bin"
    native = measured([
        str(root / "run"), "cargo-zisk", "verify", "--proof", str(proof),
        "--plonk-vk", str(work / "trusted-plonk.json"),
        "--setup-vk", str(work / "trusted-setup.json"), "-v",
    ], env, work / "native.log", 30, 512 * 2**20)
    native["macos_arithmetic_unimplemented"] = "Fr_rawToMontgomery() not implemented for macOS" in (work / "native.log").read_text()
    print("Native verification:", json.dumps(native), flush=True)
    result = {
        "zisk": versions["zisk"], "native": native,
        "proof_generated": False, "source_state_admitted": False,
        "proof_kind": "upstream wrapped PLONK fixture", "downloads": False,
        "fixture_manifest_sha256": digest(HERE / "verification-fixture.json"),
    }
    (work / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    exported = measured([
        str(root / "run"), "cargo-zisk-dev", "export-solidity-calldata",
        "--proof", str(proof), "--output", str(work / "calldata.json"),
    ], env, work / "export.log", 15, 256 * 2**20)
    if exported["exit_code"] != 0 or exported["stopped_for"]:
        raise RuntimeError("Calldata export failed; inspect retained logs")
    fixture = json.loads((work / "calldata.json").read_text())
    for name in ["programVK", "rootCVadcopFinal"]:
        if fixture[name] != trusted[name]:
            raise RuntimeError(f"Unexpected fixture statement: {name}")
    for name, size in [("publicValues", 512), ("proofBytes", 768)]:
        if not re.fullmatch(r"0x[0-9a-fA-F]+", fixture[name]) or len(bytes.fromhex(fixture[name][2:])) != size:
            raise RuntimeError(f"Unexpected fixture encoding: {name}")
    evm = work / "evm"
    (evm / "src").mkdir(parents=True)
    (evm / "test").mkdir()
    for name in ["ZiskVerifier.sol", "IZiskVerifier.sol", "PlonkVerifier.sol"]:
        shutil.copy2(source / "zisk-contracts" / name, evm / "src" / name)
    (evm / "foundry.toml").write_text('[profile.default]\nsrc = "src"\ntest = "test"\nevm_version = "cancun"\noptimizer = true\noptimizer_runs = 200\n')
    test = (HERE / "verification.t.sol.in").read_text()
    for name, value in {
        "PROGRAM": fixture["programVK"], "ROOT": expected_root,
        "PUBLICS": fixture["publicValues"][2:], "PROOF": fixture["proofBytes"][2:],
    }.items():
        test = test.replace("__" + name + "__", value)
    (evm / "test/ZiskProof.t.sol").write_text(test)
    verified = measured([
        forge, "test", "--root", str(evm), "--offline", "--use", str(args.solc.resolve()),
        "-j", "1", "-vv",
    ], env, work / "evm.log", 90, 768 * 2**20)
    tests = re.findall(r"\[PASS\]\s+(test\w+)\(\) \(gas: (\d+)\)", (work / "evm.log").read_text())
    result.update({"export": exported, "evm": verified, "tests": [{"name": name, "test_gas": int(gas)} for name, gas in tests]})
    (work / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    if verified["exit_code"] or verified["stopped_for"] or len(tests) != 5:
        raise RuntimeError("Genuine-proof EVM verification checks failed")
    print("Local EVM verification:", json.dumps(verified), flush=True)
    print("Passed", len(tests), "genuine-proof checks; no new proof was generated", flush=True)


if __name__ == "__main__":
    main()
