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


@unittest.skipUnless(os.environ.get("ZUTTI_SPV_BINARY"), "set ZUTTI_SPV_BINARY for the real CLI integration test")
class ArchiveIntegrationTests(unittest.TestCase):
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
