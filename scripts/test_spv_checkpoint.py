import json
from pathlib import Path
import tempfile
import unittest

from spv_checkpoint import export_checkpoint
from spv_headers import HEADER_BYTES


class CheckpointExportTests(unittest.TestCase):
    def node(self):
        import hashlib
        root = Path(__file__).resolve().parent.parent
        blocks = [bytes.fromhex((root / path).read_text().strip())[:HEADER_BYTES] for path in
                  ["tests/fixtures/block-test-0.txt"] + [f"tests/fixtures/spv/testnet-{h}.hex" for h in range(1, 5)]]
        hashes = [hashlib.sha256(hashlib.sha256(raw).digest()).digest()[::-1].hex() for raw in blocks]
        class Node:
            def call(self, method, params):
                if method == "getblockchaininfo":
                    return {"chain": "test", "blocks": 4}
                if method == "getblockhash":
                    return hashes[params[0]]
                if method == "getblockheader":
                    return blocks[hashes.index(params[0])].hex()
                raise AssertionError(method)
        return Node(), blocks

    def test_context_is_pinned_and_never_marked_as_proved(self):
        node, blocks = self.node()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "checkpoint"
            result = export_checkpoint(node, 3, output)
            self.assertEqual((output / "headers.bin").read_bytes(), b"".join(blocks[:4]))
            self.assertEqual(result["context_headers"], 4)
            self.assertEqual(result["work_basis"], "retained_context")
            self.assertFalse(result["proof_generated"])
            self.assertGreater(int(result["work"], 16), 0)
            self.assertEqual(json.loads((output / "export.json").read_text()), result)
            with self.assertRaises(ValueError):
                export_checkpoint(node, 3, output)

    def test_invalid_height_and_changed_branch_publish_nothing(self):
        node, _ = self.node()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "checkpoint"
            for height in (0, True, 500_000_000):
                with self.assertRaises(ValueError):
                    export_checkpoint(node, height, output)
            original = node.call
            reads = 0
            def changed(method, params):
                nonlocal reads
                if method == "getblockhash" and params == [3]:
                    reads += 1
                    if reads == 3:
                        return "00" * 32
                return original(method, params)
            node.call = changed
            with self.assertRaisesRegex(ValueError, "branch changed"):
                export_checkpoint(node, 3, output)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
