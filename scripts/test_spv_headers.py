import hashlib
from pathlib import Path
import unittest

from spv_headers import HEADER_BYTES, header


class HeaderTests(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).resolve().parent.parent
        self.blocks = [bytes.fromhex((root / path).read_text().strip()) for path in (
            "tests/fixtures/block-test-0.txt", "tests/fixtures/spv/testnet-1.hex",
        )]

    def block_hash(self, block):
        return hashlib.sha256(hashlib.sha256(block[:HEADER_BYTES]).digest()).digest()[::-1].hex()

    def test_real_genesis_and_first_header_extract_with_exact_identities(self):
        parent = "0" * 64
        for block in self.blocks:
            identity = self.block_hash(block)
            self.assertEqual(header(block.hex(), identity, parent), block[:HEADER_BYTES])
            parent = identity

    def test_changed_bytes_wrong_hash_parent_and_solution_prefix_fail(self):
        first = self.blocks[1]
        identity = self.block_hash(first)
        parent = self.block_hash(self.blocks[0])
        cases = [(first.hex(), "f" * 64, parent), (first.hex(), identity, "0" * 64),
                 (first[:100].hex(), identity, parent), ("invalid", identity, parent)]
        changed = bytearray(first)
        changed[140] ^= 1
        cases.append((changed.hex(), identity, parent))
        changed = bytearray(first)
        changed[60] ^= 1
        cases.append((changed.hex(), identity, parent))
        for values in cases:
            with self.subTest(values=values[1:]), self.assertRaises(ValueError):
                header(*values)


if __name__ == "__main__":
    unittest.main()
