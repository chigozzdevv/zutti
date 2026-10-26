import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


RUNNER = Path(__file__).with_name("run.py")


class SupervisorTests(unittest.TestCase):
    def invoke(self, directory, code, seconds=5):
        log = Path(directory) / "job.log"
        result = subprocess.run(
            [sys.executable, str(RUNNER), "--seconds", str(seconds), "--rss-mib", "128",
             "--log", str(log), "--", sys.executable, "-c", code],
            capture_output=True, text=True, timeout=20,
        )
        report = json.loads(log.with_suffix(".log.json").read_text())
        return result, report

    def test_success_and_memory_limit_are_reported_separately(self):
        with tempfile.TemporaryDirectory() as directory:
            result, report = self.invoke(directory, "print('completed')")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(report["status"], "completed")
        with tempfile.TemporaryDirectory() as directory:
            result, report = self.invoke(directory, "import time; data=bytearray(b'x'*(160*1024*1024)); time.sleep(10)")
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(report["status"], "memory_limit")

    def test_timeout_stops_a_term_resistant_child_and_preserves_other_processes(self):
        sentinel = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], start_new_session=True)
        try:
            with tempfile.TemporaryDirectory() as directory:
                child = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)"
                code = f"import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{child!r}]); time.sleep(30)"
                result, report = self.invoke(directory, code, seconds=1)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(report["status"], "timeout", result.stdout + result.stderr)
                self.assertIsNone(report["error"])
                self.assertIsNone(sentinel.poll())
        finally:
            sentinel.terminate()
            sentinel.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
