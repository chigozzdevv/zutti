import unittest
import json
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from spv_source import target, check_branch


class SourceTargetTests(unittest.TestCase):
    def test_confirmations_are_left_for_admission_and_policy_is_not_extended(self):
        status = {"confirmations": 6, "valid_through": 200}
        self.assertEqual(target(status, {"chain": "test", "blocks": 100}), 94)
        self.assertEqual(target(status, {"chain": "test", "blocks": 300}), 194)
        self.assertEqual(target(status, {"chain": "test", "blocks": 2}), 0)

    def test_wrong_network_and_invalid_policy_cannot_select_a_target(self):
        for status, info in [
            ({"confirmations": 0, "valid_through": 200}, {"chain": "test", "blocks": 100}),
            ({"confirmations": 6, "valid_through": 6}, {"chain": "test", "blocks": 100}),
            ({"confirmations": 6, "valid_through": 200}, {"chain": "main", "blocks": 100}),
            ({"confirmations": 6, "valid_through": 200}, {"chain": "test", "blocks": True}),
        ]:
            with self.assertRaises(ValueError):
                target(status, info)

    def test_a_node_behind_verified_or_pending_work_stops_before_proving(self):
        for current in [
            {"verified_height": 10, "pending_height": None},
            {"verified_height": None, "pending_height": 10},
        ]:
            with self.assertRaises(ValueError):
                check_branch(current, {"chain": "test", "blocks": 9}, None)

    def test_checkpoint_branch_is_checked_before_any_verified_batch_exists(self):
        current = {"checkpoint_height": 10, "checkpoint_tip": "11" * 32,
                   "verified_height": None, "pending_height": None}
        class Node:
            def call(self, method, params):
                self.parameters = method, params
                return "22" * 32
        node = Node()
        with self.assertRaisesRegex(ValueError, "source branch changed"):
            check_branch(current, {"chain": "test", "blocks": 20}, node)
        self.assertEqual(node.parameters, ("getblockhash", [10]))


@unittest.skipUnless(os.environ.get("ZUTTI_SPV_BINARY"), "set ZUTTI_SPV_BINARY for the real CLI integration test")
class ArchiveIntegrationTests(unittest.TestCase):
    def test_checkpoint_worker_starts_after_the_pinned_context_and_retains_pending_progress(self):
        root = Path(__file__).resolve().parent.parent
        fixture = root / "deploy/source/checkpoint"
        profile = json.loads((fixture / "profile.json").read_text())
        for name, artifact in profile["artifacts"].items():
            value = (fixture / name).read_bytes()
            self.assertEqual(len(value), artifact["bytes"])
            self.assertEqual(hashlib.sha256(value).hexdigest(), artifact["sha256"])
        checkpoint = profile["checkpoint"]
        first = checkpoint["height"] + 1
        raw = (fixture / "following-headers.bin").read_bytes()
        headers = [raw[offset:offset + 1487] for offset in range(0, len(raw), 1487)]
        hashes = {first + i: hashlib.sha256(hashlib.sha256(value).digest()).digest()[::-1].hex()
                  for i, value in enumerate(headers)}
        hashes[checkpoint["height"]] = checkpoint["rpc_display_hash"]
        end = first + len(headers) - 1
        requested = []

        class Archive(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                method, params = request["method"], request["params"]
                if method == "getblockchaininfo":
                    result = {"chain": "test", "blocks": end + 10}
                elif method == "getblockhash":
                    requested.append(params[0])
                    result = hashes[params[0]]
                elif method == "getblock":
                    height = next(h for h, value in hashes.items() if value == params[0])
                    result = headers[height - first].hex()
                else:
                    raise ValueError("unexpected archive operation")
                body = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Archive)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                directory = Path(directory)
                binary = Path(os.environ["ZUTTI_SPV_BINARY"]).resolve()
                state = directory / "state"
                def invoke(*args):
                    result = subprocess.run([str(binary), *map(str, args)], check=True, capture_output=True, text=True, timeout=90)
                    return json.loads(result.stdout.strip().splitlines()[-1])
                recreated = directory / "checkpoint"
                result = invoke("checkpoint", "--headers", fixture / "headers.bin", "--height", str(checkpoint["height"]),
                                "--work", checkpoint["work"], "--out", recreated)
                self.assertEqual(result["checkpoint_id"], checkpoint["commitment"])
                self.assertEqual((recreated / "checkpoint.bin").read_bytes(), (fixture / "checkpoint.bin").read_bytes())
                program = root / "crates/prover/spv/fixtures/genesis-coverage/program.elf"
                invoke("bootstrap", "init", "--state", state, "--program", program, "--checkpoint", recreated / "checkpoint.bin",
                       "--confirmations", "10", "--window-blocks", "128", "--valid-through", "5000000")
                cookie = directory / "cookie"
                cookie.write_text("archive:public-fixture")
                cookie.chmod(0o600)
                command = [sys.executable, str(root / "scripts/spv_source.py"), "--binary", str(binary), "--state", str(state),
                           "--work", str(directory / "work"), "--endpoint", f"http://127.0.0.1:{server.server_port}", "--cookie", str(cookie), "--batch-size", "20"]
                result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=90)
                self.assertEqual(json.loads(result.stdout.strip().splitlines()[-1])["state"], "prepared")
                current = invoke("bootstrap", "status", "--state", state)
                self.assertEqual(current["bootstrap"], "checkpoint")
                self.assertEqual(current["checkpoint_height"], checkpoint["height"])
                self.assertEqual(current["pending_height"], end)
                self.assertEqual(current["verified_batches"], 0)
                self.assertIsNone(current["verified_height"])
                self.assertFalse(current["settlement_authority"])
                subprocess.run(command, check=True, capture_output=True, text=True, timeout=90)
                self.assertEqual(invoke("bootstrap", "status", "--state", state), current)
                self.assertEqual(min(requested), checkpoint["height"])
        finally:
            server.shutdown()
            thread.join(timeout=5)
            server.server_close()

    def test_worker_prepares_real_headers_without_promoting_them_to_verified_progress(self):
        root = Path(__file__).resolve().parent.parent
        blocks = [bytes.fromhex((root / path).read_text().strip()) for path in
                  ["tests/fixtures/block-test-0.txt"] + [f"tests/fixtures/spv/testnet-{h}.hex" for h in range(1, 5)]]
        hashes = [hashlib.sha256(hashlib.sha256(b[:1487]).digest()).digest()[::-1].hex() for b in blocks]

        class Archive(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                method, params = request["method"], request["params"]
                if method == "getblockchaininfo":
                    result = {"chain": "test", "blocks": 4}
                elif method == "getblockhash":
                    result = hashes[params[0]]
                elif method == "getblock":
                    result = blocks[hashes.index(params[0])].hex()
                else:
                    raise ValueError("unexpected archive operation")
                body = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Archive)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                directory = Path(directory)
                binary = Path(os.environ["ZUTTI_SPV_BINARY"]).resolve()
                state = directory / "state"
                genesis = directory / "genesis.bin"
                genesis.write_bytes(blocks[0][:1487])
                fixture = root / "crates/prover/spv/fixtures/genesis-coverage"
                def invoke(*args):
                    result = subprocess.run([str(binary), *map(str, args)], check=True, capture_output=True, text=True, timeout=90)
                    return json.loads(result.stdout.strip().splitlines()[-1])
                invoke("bootstrap", "init", "--state", state, "--program", fixture / "program.elf", "--genesis", genesis,
                       "--confirmations", "1", "--window-blocks", "3", "--valid-through", "4")
                invoke("bootstrap", "prepare", "--state", state)
                invoke("bootstrap", "accept", "--state", state, "--batch", "0", "--receipt", fixture / "receipt.bin")
                cookie = directory / "cookie"
                cookie.write_text("archive:public-fixture")
                cookie.chmod(0o600)
                command = [sys.executable, str(root / "scripts/spv_source.py"), "--binary", str(binary), "--state", str(state),
                           "--work", str(directory / "work"), "--endpoint", f"http://127.0.0.1:{server.server_port}", "--cookie", str(cookie), "--batch-size", "3"]
                result = subprocess.run(command, check=True, capture_output=True, text=True, timeout=90)
                self.assertEqual(json.loads(result.stdout.strip().splitlines()[-1])["state"], "prepared")
                current = invoke("bootstrap", "status", "--state", state)
                self.assertEqual(current["verified_height"], 0)
                self.assertEqual(current["pending_height"], 3)
                self.assertFalse(current["settlement_authority"])
                subprocess.run(command, check=True, capture_output=True, text=True, timeout=90)
                self.assertEqual(invoke("bootstrap", "status", "--state", state), current)
        finally:
            server.shutdown()
            thread.join(timeout=5)
            server.server_close()


if __name__ == "__main__":
    unittest.main()
