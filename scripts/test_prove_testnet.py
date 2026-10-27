import argparse
import json
from pathlib import Path
import tempfile
import unittest

from prove_testnet import save, submit


class BudgetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.args = argparse.Namespace(ledger=root / "approval.json", config=root / "config.env",
            binary=root / "prover", job=root / "job.json", name="source-1")
        self.args.config.write_text("BOUNDLESS_MAX_PRICE_WEI=60\n")
        self.args.config.chmod(0o600)
        save(self.args.job, {"visibility":"public","image_id":"11"*32,"stdin_sha256":"22"*32,
                            "journal_sha256":"33"*32,"cycles":100})
        save(self.args.ledger,{"network":"base","automatic_resubmission":False,
            "testnet_witnesses_public":True,"maximum_requests":2,"per_request_cap_wei":"60",
            "total_cap_wei":"100","requests":[]})
        self.args.binary.write_text('#!/usr/bin/env python3\nimport json,os,sys\np=sys.argv[sys.argv.index("--request")+1]\nfd=os.open(p,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)\nwith os.fdopen(fd,"w") as f:json.dump({"max_price_wei":"60","chain_id":8453,"request_id":"0x1234","expires_at":100},f)\n')
        self.args.binary.chmod(0o700)

    def tearDown(self):
        self.temporary.cleanup()

    def test_attempt_reserves_the_cap_and_cannot_be_renamed_or_resubmitted(self):
        submit(self.args)
        ledger=json.loads(self.args.ledger.read_text())
        self.assertEqual(ledger["requests"][0]["reserved_wei"],"60")
        self.assertEqual(ledger["requests"][0]["state"],"submitted")
        for name in ("source-1","source-2"):
            self.args.name=name
            with self.assertRaises(ValueError):submit(self.args)
        self.assertEqual(len(json.loads(self.args.ledger.read_text())["requests"]),1)

    def test_run_cap_prevents_a_second_distinct_request(self):
        submit(self.args)
        self.args.name="binding"
        job=json.loads(self.args.job.read_text());job["journal_sha256"]="44"*32
        self.args.job=self.args.job.with_name("binding.json");save(self.args.job,job)
        with self.assertRaises(ValueError):submit(self.args)
        self.assertFalse((self.args.ledger.parent/"binding-request.json").exists())

    def test_unknown_submission_stays_reserved_and_requires_inspection(self):
        self.args.binary.write_text("#!/bin/sh\nexit 1\n")
        with self.assertRaises(RuntimeError):submit(self.args)
        record=json.loads(self.args.ledger.read_text())["requests"][0]
        self.assertEqual(record["state"],"inspect_required")
        self.assertEqual(record["reserved_wei"],"60")
        with self.assertRaises(ValueError):submit(self.args)

    def test_price_cap_and_public_transport_are_required_before_attempting(self):
        for change in ("price","visibility"):
            if change=="price":self.args.config.write_text("BOUNDLESS_MAX_PRICE_WEI=61\n")
            else:
                self.args.config.write_text("BOUNDLESS_MAX_PRICE_WEI=60\n")
                job=json.loads(self.args.job.read_text());job["visibility"]="private";save(self.args.job,job)
            with self.assertRaises(ValueError):submit(self.args)
        self.assertEqual(json.loads(self.args.ledger.read_text())["requests"],[])


if __name__=="__main__":unittest.main()
