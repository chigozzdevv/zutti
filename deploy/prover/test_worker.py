import argparse
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(f"{name}.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


worker = load("run")
builder = load("prepare")


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.job = self.root / "job"
        self.job.mkdir()
        for name in worker.FILES:
            (self.job / name).write_bytes(b"public input")
        (self.job / "job.json").write_text(json.dumps({
            "version": 1, "visibility": "public", "image_id": "b" * 64, "cycles": 100,
        }))
        self.args = argparse.Namespace(
            job=self.job, output=self.root / "output", cycle_limit=100, segment_po2=20,
            max_seconds=30, min_free_gib=1, source_prover="source-prover", r0vm="r0vm",
        )
        self.calls = []
        self.network = "testnet"
        self.reject_proof = False
        self.build = self.root / "build.json"
        self.build.write_text('{"commit":"test fixture"}')

    def command(self, arguments, deadline, log=None):
        self.calls.append(arguments)
        if arguments[1] == "inspect-job":
            return json.dumps({
                "job_id": "a" * 64, "image_id": "b" * 64,
                "network": self.network, "source_only": True,
            }).encode()
        if arguments[1] == "prove":
            Path(arguments[arguments.index("--out") + 1]).write_bytes(b"mock receipt")
        if arguments[1] == "verify":
            if self.reject_proof:
                raise RuntimeError("receipt rejected")
            Path(arguments[arguments.index("--seal") + 1]).write_bytes(bytes(260))
        return b"{}"

    def run_worker(self):
        with patch.object(worker, "command", self.command), patch.object(worker, "BUILD_MANIFEST", self.build), contextlib.redirect_stdout(io.StringIO()):
            worker.run(self.args)

    def test_new_proofs_require_groth16_and_are_verified_before_completion(self):
        self.run_worker()
        proving = next(call for call in self.calls if call[1] == "prove")
        self.assertIn("--groth16", proving)
        self.assertIn("--cycle-limit", proving)
        result = self.args.output / ("a" * 64)
        self.assertTrue((result / "timing.json").exists())
        self.assertTrue((result / "hardware.json").exists())
        self.assertEqual((result / "seal.bin").stat().st_size, 260)

    def test_resume_verifies_against_the_submitted_job(self):
        result = self.args.output / ("a" * 64)
        result.mkdir(parents=True)
        (result / "source.proof").write_bytes(b"mock receipt")
        self.run_worker()
        self.assertFalse(any(call[1] == "prove" for call in self.calls))
        verification = next(call for call in self.calls if call[1] == "verify")
        self.assertIn("--job", verification)
        self.assertTrue(json.loads((result / "verified.json").read_text())["resumed"])
        self.reject_proof = True
        with self.assertRaisesRegex(RuntimeError, "receipt rejected"):
            self.run_worker()
        self.assertFalse((result / "verified.json").exists())

    def test_mainnet_is_rejected_before_proving(self):
        self.network = "mainnet"
        with self.assertRaisesRegex(ValueError, "Testnet"):
            self.run_worker()
        self.assertEqual(len(self.calls), 1)

    def test_symlinks_and_private_manifests_are_rejected(self):
        (self.job / "stdin.bin").unlink()
        (self.job / "stdin.bin").symlink_to(self.job / "statement.bin")
        with self.assertRaises(OSError):
            self.run_worker()
        (self.job / "stdin.bin").unlink()
        (self.job / "stdin.bin").write_bytes(b"input")
        (self.job / "job.json").write_text('{"version":1,"visibility":"private","cycles":100}')
        with self.assertRaisesRegex(ValueError, "public source"):
            self.run_worker()
        self.assertFalse(self.calls)

    def test_limits_fail_before_starting_a_worker(self):
        self.args.cycle_limit = worker.MAX_CYCLES + 1
        with self.assertRaisesRegex(ValueError, "limits"):
            self.run_worker()
        self.assertFalse(self.calls)

    def test_expired_processes_are_terminated(self):
        started = time.monotonic()
        with self.assertRaises(TimeoutError):
            worker.command([sys.executable, "-c", "import time; time.sleep(30)"], started + 0.1)
        self.assertLess(time.monotonic() - started, 6)


class BuildContextTests(unittest.TestCase):
    def test_only_committed_sources_and_matching_programs_are_copied(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            for name in builder.ROOT_FILES:
                (repo / name).write_text("public source")
            (repo / "deploy/prover").mkdir(parents=True)
            for name in ("Dockerfile", "run.py"):
                (repo / "deploy/prover" / name).write_text("public code")
            manifest = {}
            programs = root / "programs"
            for name in builder.NAMES:
                program = programs / name / "program.elf"
                program.parent.mkdir(parents=True)
                program.write_bytes(name.encode())
                manifest[name] = {"sha256": builder.hashlib.sha256(name.encode()).hexdigest()}
            methods = repo / "crates/prover/methods"
            methods.mkdir(parents=True)
            (methods / "programs.json").write_text(json.dumps(manifest))
            subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
            subprocess.run([
                "git", "-C", str(repo), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                "commit", "-qm", "test fixture",
            ], check=True)
            (repo / ".env").write_text("secret")
            (repo / ".data").mkdir()
            (repo / ".data/wallet.key").write_text("secret")
            context = root / "context"
            builder.prepare(repo, programs, context)
            self.assertFalse((context / "repo/.env").exists())
            self.assertFalse((context / "repo/.git").exists())
            self.assertFalse((context / "repo/.data").exists())
            (programs / "source/program.elf").write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "release manifest"):
                builder.prepare(repo, programs, root / "tampered")
            self.assertFalse((root / "tampered").exists())


if __name__ == "__main__":
    unittest.main()
