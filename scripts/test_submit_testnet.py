import io
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from submit_testnet import Rpc


class RpcTests(unittest.TestCase):
    @patch("submit_testnet.time.sleep")
    @patch("submit_testnet.urllib.request.urlopen")
    def test_read_recovers_from_connection_reset(self, open_url, sleep):
        open_url.side_effect = [URLError(ConnectionResetError()),
                                io.BytesIO(b'{"result":"0xaa36a7"}')]
        self.assertEqual(Rpc("https://rpc.invalid").call("eth_chainId", []), "0xaa36a7")
        self.assertEqual(open_url.call_count, 2)
        sleep.assert_called_once_with(0.5)

    @patch("submit_testnet.time.sleep")
    @patch("submit_testnet.urllib.request.urlopen")
    def test_read_retries_are_bounded(self, open_url, sleep):
        open_url.side_effect = URLError("offline")
        with self.assertRaises(URLError):
            Rpc("https://rpc.invalid").call("eth_getTransactionReceipt", ["0x01"])
        self.assertEqual(open_url.call_count, 3)
        self.assertEqual(sleep.call_count, 2)

    @patch("submit_testnet.urllib.request.urlopen")
    def test_broadcast_is_never_retried(self, open_url):
        open_url.side_effect = URLError("unknown broadcast outcome")
        with self.assertRaises(URLError):
            Rpc("https://rpc.invalid").call("eth_sendRawTransaction", ["0x01"])
        self.assertEqual(open_url.call_count, 1)

    @patch("submit_testnet.urllib.request.urlopen")
    def test_rpc_rejection_is_not_retried(self, open_url):
        open_url.return_value = io.BytesIO(b'{"error":{"code":-32000,"message":"reverted"}}')
        with self.assertRaisesRegex(RuntimeError, "eth_call failed"):
            Rpc("https://rpc.invalid").call("eth_call", [])
        self.assertEqual(open_url.call_count, 1)

    @patch("submit_testnet.urllib.request.urlopen")
    def test_access_denied_is_not_retried(self, open_url):
        error = HTTPError("https://rpc.invalid", 403, "Forbidden", {}, None)
        open_url.side_effect = error
        try:
            with self.assertRaises(HTTPError):
                Rpc("https://rpc.invalid").call("eth_chainId", [])
        finally:
            error.close()
        self.assertEqual(open_url.call_count, 1)


if __name__ == "__main__":
    unittest.main()
