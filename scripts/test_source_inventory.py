import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid

from source_inventory import COUNT_FIELDS, InventoryStore, add_row, mongodb_config, read_line, report, stop, validate_row, write_all
from source_rpc import Rpc, private_json


def row(height=0, parent="0" * 64):
    counts = dict.fromkeys(COUNT_FIELDS, 0)
    counts.update(transactions=1, transparent_outputs=1)
    return {
        "schema": 1, "height": height, "block_hash": f"{height + 1:064x}", "parent_hash": parent,
        "branch": "Sprout", "branch_id": "00000000", "block_bytes": 1611, "counts": counts,
        "versions": {"1": 1}, "consensus_verified": False, "proof_generated": False,
    }


class InventoryValidationTests(unittest.TestCase):
    def test_counterfeit_authority_and_mismatched_ancestry_fail(self):
        valid = row()
        validate_row(valid, 0, "0" * 64)
        for field, value in (("schema", True), ("height", 1), ("parent_hash", "7" * 64), ("proof_generated", True),
                             ("consensus_verified", True), ("block_hash", "x" * 64), ("block_bytes", 2_000_001)):
            altered = dict(valid, **{field: value})
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_row(altered, 0, "0" * 64)

    def test_boolean_negative_and_inconsistent_counts_fail(self):
        for value in (True, -1, 2_000_001):
            changed = row()
            changed["counts"]["transparent_inputs"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_row(changed, 0, "0" * 64)
        changed = row()
        changed["versions"] = {"1": 2}
        with self.assertRaises(ValueError):
            validate_row(changed, 0, "0" * 64)

    def test_large_output_counts_are_not_artificially_limited_to_transaction_count(self):
        value = row()
        value["counts"]["transparent_outputs"] = 150_000
        validate_row(value, 0, "0" * 64)

    def test_group_totals_and_samples_are_deterministic(self):
        totals = dict.fromkeys(("blocks", "block_bytes") + COUNT_FIELDS, 0)
        groups = {}
        for height in range(10):
            value = row(height)
            add_row(totals, groups, value)
        self.assertEqual(totals["blocks"], 10)
        self.assertEqual(totals["block_bytes"], 16110)
        self.assertEqual(totals["transactions"], 10)
        group = next(iter(groups.values()))
        self.assertEqual(group["blocks"], 10)
        self.assertEqual(len(group["samples"]), 3)
        self.assertEqual([item["height"] for item in group["largest"]], [0, 1, 2])

    def test_framing_enforces_size_and_deadline_even_without_a_newline(self):
        programs = [("import time; print('x'*100,flush=True); time.sleep(2)", 16, RuntimeError),
                    ("import sys,time; sys.stdout.write('x');sys.stdout.flush();time.sleep(2)", 100, TimeoutError)]
        for program, limit, failure in programs:
            child = subprocess.Popen([sys.executable, "-c", program], stdout=subprocess.PIPE, stdin=subprocess.PIPE)
            try:
                with self.assertRaises(failure):
                    read_line(child, limit, 0.3)
            finally:
                stop(child)

    def test_partial_pipe_writes_do_not_truncate_block_frames(self):
        class PartialWriter:
            def __init__(self):
                self.data = bytearray()

            def write(self, value):
                count = min(3, len(value))
                self.data.extend(value[:count])
                return count

        writer = PartialWriter()
        write_all(writer, b"a complete block frame")
        self.assertEqual(writer.data, b"a complete block frame")

    def test_reports_never_convert_counts_into_claimed_cycle_totals(self):
        state = {"_id": "test", "binding": {"target_height": 9}, "next_height": 2,
                 "last_hash": "1" * 64, "totals": {"blocks": 2}, "groups": {}}
        value, encoded = report(state, 2, 1.0)
        self.assertEqual(value["scan_eta_seconds_at_session_rate"], 4.0)
        self.assertIsNone(value["total_cycle_estimate"])
        self.assertFalse(value["complete"])
        self.assertFalse(value["proof_generated"])
        self.assertLess(len(encoded), 1_000_000)

    def test_private_configuration_rejects_symlinks_and_public_permissions(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text('{"value":1}')
            path.chmod(0o600)
            self.assertEqual(private_json(path), {"value": 1})
            link = Path(directory) / "link.json"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                private_json(link)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                private_json(path)

    def test_rpc_rejects_external_endpoints_before_reading_a_cookie(self):
        for endpoint in ("https://localhost", "http://example.com", "http://user:password@localhost"):
            with self.assertRaises(ValueError):
                Rpc(endpoint, Path("does-not-exist"))

    def test_existing_storage_profile_is_read_without_copying_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "storage.env"
            path.write_text('ZUTTI_MONGODB_URI="mongodb://inventory:example@localhost:27017/"\nZUTTI_MONGODB_DATABASE=zutti\n')
            path.chmod(0o600)
            self.assertEqual(mongodb_config(path), {"uri": "mongodb://inventory:example@localhost:27017/", "database": "zutti"})
            with path.open("a") as out:
                out.write("ZUTTI_MONGODB_DATABASE=other\n")
            with self.assertRaises(ValueError):
                mongodb_config(path)


@unittest.skipUnless(os.environ.get("ZUTTI_INVENTORY_MONGO_CONFIG"), "requires the isolated MongoDB integration profile")
class InventoryMongoTests(unittest.TestCase):
    def setUp(self):
        self.prefix = "source_inventory_test_" + uuid.uuid4().hex + "_"
        config = mongodb_config(Path(os.environ["ZUTTI_INVENTORY_MONGO_CONFIG"]), os.environ.get("ZUTTI_INVENTORY_MONGO_CONTAINER"))
        self.store = InventoryStore(config, self.prefix)
        self.binding = {"schema": 1, "network": "mainnet", "start_height": 0,
                        "target_height": 9, "target_hash": "a" * 64, "scanner_sha256": "b" * 64}
        self.store.open("test", self.binding, "0" * 64)

    def tearDown(self):
        self.store.blocks.drop()
        self.store.runs.drop()
        self.store.close()

    def test_crash_between_rows_and_frontier_recovers_without_double_counting(self):
        value = row()
        self.store.blocks.insert_one(dict(value, _id="test:0", run_id="test"))
        self.store.commit([value], 1.0)
        state = self.store.open("test", self.binding, "0" * 64)
        self.assertEqual(state["next_height"], 1)
        self.assertEqual(state["totals"]["blocks"], 1)
        self.assertEqual(self.store.blocks.count_documents({"run_id": "test"}), 1)

    def test_conflicting_persisted_row_cannot_be_overwritten(self):
        from pymongo.errors import BulkWriteError
        changed = dict(row(), block_hash="7" * 64, _id="test:0", run_id="test")
        self.store.blocks.insert_one(changed)
        with self.assertRaises(BulkWriteError):
            self.store.commit([row()], 1.0)
        self.assertEqual(self.store.runs.find_one({"_id": "test"})["next_height"], 0)

    def test_changed_program_or_target_cannot_resume_the_same_inventory(self):
        for field in ("scanner_sha256", "target_hash"):
            changed = dict(self.binding, **{field: "c" * 64})
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.store.open("test", changed, "0" * 64)

    def test_stale_worker_cannot_increment_the_frontier_twice(self):
        stale = copy.deepcopy(self.store.state)
        self.store.commit([row()], 1.0)
        self.store.state = stale
        with self.assertRaises(RuntimeError):
            self.store.commit([row()], 1.0)
        self.assertEqual(self.store.runs.find_one({"_id": "test"})["totals"]["blocks"], 1)


if __name__ == "__main__":
    unittest.main()
