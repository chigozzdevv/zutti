import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from ready_testnet import check_authorization, program_inputs


class ReadinessTests(unittest.TestCase):
    def test_closed_or_inapplicable_proof_approval_blocks_new_quotes(self):
        programs = {'admission': {'image_id': '11' * 32}}
        ledger = {'network': 'base', 'automatic_resubmission': False, 'testnet_witnesses_public': True,
                  'maximum_requests': None, 'per_request_cap_wei': '60', 'total_cap_wei': '100', 'requests': [],
                  'scope': {'kind': 'single_testnet_swap', 'completed': False, 'programs': ['11' * 32]}}
        check_authorization(ledger, programs)
        for changes in ({'completed': True}, {'programs': ['22' * 32]}, {'kind': 'benchmark'}):
            with self.assertRaises(ValueError):
                check_authorization({**ledger, 'scope': {**ledger['scope'], **changes}}, programs)
        with self.assertRaises(ValueError):
            check_authorization({**ledger, 'requests': [{'reserved_wei': '60'}]}, programs)
        with self.assertRaises(ValueError):
            check_authorization({**ledger, 'maximum_requests': 0}, programs)

    def test_proving_programs_are_pinned_before_quotes_are_admitted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            deployment = {}
            for index, kind in enumerate(('source', 'admission', 'binding', 'pool'), 1):
                artifact = root / 'programs' / kind
                artifact.mkdir(parents=True)
                data = bytes([index])
                (artifact / 'program.elf').write_bytes(data)
                deployment[f'{kind}_image'] = f'{index:02x}' * 32
                (artifact / 'program.json').write_text(json.dumps({
                    'image_id': deployment[f'{kind}_image'],
                    'program_sha256': hashlib.sha256(data).hexdigest(),
                }))
            self.assertEqual(len(program_inputs({'root': str(root)}, deployment)), 4)
            changed = {**deployment, 'admission_image': 'ff' * 32}
            with self.assertRaises(ValueError):
                program_inputs({'root': str(root)}, changed)
            (root / 'programs/pool/program.elf').write_bytes(b'changed')
            with self.assertRaises(ValueError):
                program_inputs({'root': str(root)}, deployment)


if __name__ == '__main__':
    unittest.main()
