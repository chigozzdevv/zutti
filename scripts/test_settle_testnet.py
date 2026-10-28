import json
from pathlib import Path
import tempfile
import unittest

from ethereum_evidence import atomic_save
from settle_testnet import Settlement, cached_execution, confirmation_depth, execution_identity


class SettlementTests(unittest.TestCase):
    def test_confirmation_depth_comes_from_the_pinned_policy(self):
        deployment = json.loads((Path(__file__).resolve().parent.parent / 'deploy/ethereum/sepolia.json').read_text())
        self.assertEqual(confirmation_depth(deployment), 10)
        for encoded in ('00', deployment['source_policy'] + '00'):
            with self.assertRaises(ValueError):
                confirmation_depth({**deployment, 'source_policy': encoded})
        changed = bytearray.fromhex(deployment['source_policy'])
        changed[43:47] = (0).to_bytes(4, 'little')
        with self.assertRaises(ValueError):
            confirmation_depth({**deployment, 'source_policy': changed.hex()})

    def test_completed_operations_return_the_retained_receipt_without_new_work(self):
        with tempfile.TemporaryDirectory() as directory:
            operation = Path(directory)
            atomic_save(operation / 'selected-offer.json', {'intent_id': '11' * 32})
            receipt = {'outcome': 'settled', 'swap_id': '22' * 32}
            atomic_save(operation / 'settled-receipt.json', receipt)
            runtime = object.__new__(Settlement)
            runtime.operation = operation
            returned = []
            runtime.return_receipt = returned.append
            runtime.execute()
            self.assertEqual(returned, [receipt])

    def test_execution_cache_requires_the_exact_job_and_completed_bounded_guest(self):
        with tempfile.TemporaryDirectory() as directory:
            job = Path(directory) / 'job.json'
            record = Path(directory) / 'execution.json'
            atomic_save(job, {'cycles': 100, 'stdin_sha256': '11' * 32})
            self.assertFalse(cached_execution(record, job))
            valid = {'job_sha256': execution_identity(job),
                     'verified': {'journal_matches': True, 'host_assumptions': 0, 'cycles': 80}}
            atomic_save(record, valid)
            self.assertTrue(cached_execution(record, job))
            for field, value in [('journal_matches', False), ('host_assumptions', 1), ('cycles', 101), ('cycles', 0)]:
                record.unlink()
                atomic_save(record, {**valid, 'verified': {**valid['verified'], field: value}})
                with self.assertRaises(ValueError):
                    cached_execution(record, job)
            record.unlink()
            atomic_save(record, valid)
            job.write_text(json.dumps({'cycles': 100, 'stdin_sha256': '22' * 32}))
            with self.assertRaises(ValueError):
                cached_execution(record, job)


if __name__ == '__main__':
    unittest.main()
