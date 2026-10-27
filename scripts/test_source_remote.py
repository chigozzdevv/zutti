import gzip
import json
import subprocess
import unittest
from unittest.mock import patch

from source_remote import RemoteSource


class SourceTransportTests(unittest.TestCase):
    def setUp(self):
        self.source = RemoteSource({"container": "source", "cookie": "/cookie",
            "control_socket": "/socket", "host": "source-host"})

    def result(self, value):
        return subprocess.CompletedProcess([], 0, gzip.compress(value), b"")

    def test_compressed_batch_preserves_complete_responses(self):
        payload = json.dumps([{"blocks": 123}, "aa"]).encode()
        with patch("source_remote.subprocess.run", return_value=self.result(payload)) as run:
            result = self.source.batch([{"method": "getblockchaininfo", "params": []},
                {"method": "getblockhash", "params": [123]}])
        self.assertEqual(result, [{"blocks": 123}, "aa"])
        self.assertEqual(len(json.loads(run.call_args.kwargs["input"])), 2)

    def test_compression_cannot_bypass_the_response_size_bound(self):
        with patch("source_remote.subprocess.run", return_value=self.result(b" " * 24_000_001)):
            with self.assertRaisesRegex(ValueError, "decompression bound"):
                self.source.call("getblockchaininfo", [])

    def test_incomplete_responses_and_ssh_failure_are_rejected(self):
        for result in (self.result(b"[]"), subprocess.CompletedProcess([], 255, b"", b"connection lost")):
            with self.subTest(code=result.returncode):
                with patch("source_remote.subprocess.run", return_value=result):
                    with self.assertRaises((ValueError, RuntimeError)):
                        self.source.call("getblockchaininfo", [])

    def test_broadcast_requires_explicit_authorization_and_is_never_retried(self):
        with patch("source_remote.subprocess.run", return_value=subprocess.CompletedProcess([], 255, b"", b"connection lost")) as run:
            with self.assertRaises(ValueError):
                self.source.call("sendrawtransaction", ["00"])
            run.assert_not_called()
            with self.assertRaises(RuntimeError):
                self.source.call("sendrawtransaction", ["00"], broadcast=True)
            self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
