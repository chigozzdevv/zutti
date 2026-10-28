import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from ethereum_evidence import Transport, atomic_save, collect


class Endpoint:
    beacon = "https://beacon.example"

    def __init__(self, period=0, wrong_hash=False):
        self.period = period
        self.wrong_hash = wrong_hash
        self.calls = []

    def rest(self, path):
        if "bootstrap" in path:
            return {"header": {"beacon": {"slot": "8192"}}}
        return {"signature_slot": str(8193 + self.period * 8192),
                "finalized_header": {"execution_block_hash": "0x1234"}}

    def request(self, url):
        self.calls.append(url)
        return [{"data": {"committee": i}} for i in range(self.period)]

    def rpc(self, method, params):
        self.calls.append((method, params))
        if method == "eth_chainId":
            return "0xaa36a7"
        if method == "eth_getBlockByHash":
            return {"hash": "0x5678" if self.wrong_hash else "0x1234"}
        return {"address": params[0], "storageProof": []}


class EvidenceTests(unittest.TestCase):
    @patch("ethereum_evidence.time.sleep")
    @patch("ethereum_evidence.urllib.request.urlopen")
    def test_rate_limited_proof_read_respects_retry_after(self, open_url, sleep):
        error = HTTPError("https://execution.example", 429, "Limited", {"Retry-After": "5"}, None)
        open_url.side_effect = [error, io.BytesIO(b'{"result":{"storageProof":[]}}')]
        transport = Transport("https://beacon.example", "https://execution.example")
        self.assertEqual(transport.rpc("eth_getProof", []), {"storageProof": []})
        self.assertEqual(transport.calls, 2)
        sleep.assert_called_once_with(5)

    @patch("ethereum_evidence.time.sleep")
    @patch("ethereum_evidence.urllib.request.urlopen")
    def test_transport_retries_remain_bounded(self, open_url, sleep):
        open_url.side_effect = URLError("offline")
        transport = Transport("https://beacon.example", "https://execution.example")
        with self.assertRaises(URLError):
            transport.rest("/eth/v1/beacon/light_client/finality_update")
        self.assertEqual(transport.calls, 3)
        self.assertEqual(sleep.call_count, 2)

    @patch("ethereum_evidence.urllib.request.urlopen")
    def test_transport_never_retries_a_broadcast(self, open_url):
        open_url.side_effect = URLError("unknown broadcast outcome")
        transport = Transport("https://beacon.example", "https://execution.example")
        with self.assertRaises(URLError):
            transport.rpc("eth_sendRawTransaction", ["0x01"])
        self.assertEqual(open_url.call_count, 1)

    def test_proofs_are_pinned_to_the_finalized_execution_hash(self):
        endpoint = Endpoint(period=1)
        evidence = collect(endpoint, {"chain_id": 11155111, "checkpoint": "0xabcd"},
                           {"pool": {"address": "0xpool", "keys": ["0xslot"]}})
        self.assertEqual(evidence["consensus"]["updates"], [{"committee": 0}])
        call = next(call for call in endpoint.calls if isinstance(call, tuple) and call[0] == "eth_getProof")
        self.assertEqual(call[1][-1], {"blockHash": "0x1234", "requireCanonical": True})

    def test_inconsistent_execution_and_excessive_checkpoint_gap_are_rejected(self):
        config = {"chain_id": 11155111, "checkpoint": "0xabcd"}
        for endpoint in (Endpoint(wrong_hash=True), Endpoint(period=65), Endpoint(period=-1)):
            with self.assertRaises(ValueError):
                collect(endpoint, config, {})

    def test_transport_does_not_accept_plaintext_or_embedded_credentials(self):
        for endpoint in ("http://example.com", "https://secret@example.com", "https://example.com?key=x"):
            with self.assertRaises(ValueError):
                Transport(endpoint, "https://execution.example")
        Transport("http://beacon.example", "https://execution.example", allow_http_beacon=True)
        with self.assertRaises(ValueError):
            Transport("https://beacon.example", "http://execution.example", allow_http_beacon=True)

    def test_saved_evidence_cannot_overwrite_an_existing_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            atomic_save(path, {"first": True})
            with self.assertRaises(FileExistsError):
                atomic_save(path, {"second": True})
            self.assertEqual(json.loads(path.read_text()), {"first": True})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
