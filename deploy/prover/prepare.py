#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


ROOT_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
NAMES = ("source", "source-update", "pool", "preparation", "history-query")


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def prepare(repo, programs, destination):
    if destination.exists():
        raise ValueError("build context already exists")
    if git(repo, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("commit tracked changes before preparing a release image")
    commit = git(repo, "rev-parse", "HEAD").decode().strip()
    manifest = json.loads(git(repo, "show", f"{commit}:crates/prover/methods/programs.json"))
    files = git(repo, "ls-tree", "-rz", "--name-only", commit, "--", *ROOT_FILES, "crates/")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="prover-context-", dir=destination.parent) as temporary:
        context = Path(temporary) / "context"
        context.mkdir(mode=0o700)
        for entry in files.decode().split("\0"):
            if not entry:
                continue
            path = Path(entry)
            if path.name == ".env.example":
                continue
            if path.is_absolute() or ".." in path.parts or any(p.startswith(".") for p in path.parts):
                raise ValueError("unexpected build source path")
            if path.suffix in (".key", ".pem", ".proof", ".witness", ".backup"):
                raise ValueError("private artifact found in tracked build sources")
            output = context / "repo" / path
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(git(repo, "show", f"{commit}:{entry}"))
        for name in NAMES:
            source = programs / name / "program.elf"
            if source.is_symlink() or not source.is_file() or not 0 < source.stat().st_size <= 32 * 1024**2:
                raise ValueError(f"invalid pinned program: {name}")
            data = source.read_bytes()
            if hashlib.sha256(data).hexdigest() != manifest[name]["sha256"]:
                raise ValueError(f"program differs from committed release manifest: {name}")
            output = context / "programs" / name / "program.elf"
            output.parent.mkdir(parents=True)
            output.write_bytes(data)
        for name in ("Dockerfile", "run.py"):
            (context / name).write_bytes(git(repo, "show", f"{commit}:deploy/prover/{name}"))
        (context / "build.json").write_text(json.dumps({
            "commit": commit, "risc0": "3.0.5", "groth16": "0.1.0", "programs": manifest,
        }, indent=2) + "\n")
        (context / ".dockerignore").write_text(".git\n.env*\n**/*.key\n**/*.pem\n")
        os.rename(context, destination)
    return commit


def main():
    parser = argparse.ArgumentParser(description="Prepare an immutable public-code prover image context")
    parser.add_argument("--programs", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    try:
        commit = prepare(repo, args.programs, args.out)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps({"context": str(args.out), "commit": commit, "uploaded": False}))


if __name__ == "__main__":
    main()
