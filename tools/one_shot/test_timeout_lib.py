"""BR-009: exercise command status, output and owned process-group termination."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

HELPER = Path(__file__).resolve().with_name("_timeout_lib.sh")


class TimeoutTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="TEST_CODE_timeout-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.worker = self.root / "worker.py"
        self.worker.write_text(
            "import os,time,signal,sys,json\n"
            "from pathlib import Path\n"
            "sub=os.fork()\n"
            "if sub==0:\n"
            " signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
            " while True:time.sleep(0.02)\n"
            "else:\n"
            " Path(sys.argv[1]).write_text(json.dumps({'child':sub,'supervisor':os.getppid()}))\n"
            " os.waitpid(sub,0)\n"
        )

    def command(self, seconds, *args):
        return ["bash", "-c", 'source "$1"; shift; with_timeout "$@"',
                "TEST_CODE", str(HELPER), str(seconds), *args]

    def group(self, seconds):
        pidfile = self.root / "pid.json"
        process = subprocess.Popen(
            self.command(seconds, sys.executable, str(self.worker), str(pidfile)),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        deadline = time.monotonic() + 3
        while not pidfile.exists():
            self.assertIsNone(process.poll(), "supervisor exited before worker startup")
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.02)
        identities = json.loads(pidfile.read_text())

        def cleanup():
            if process.poll() is None:
                os.kill(identities["supervisor"], signal.SIGTERM)
                process.communicate(timeout=3)
        self.addCleanup(cleanup)
        return process, identities

    def assert_stopped(self, pid):
        deadline = time.monotonic() + 2
        while True:
            result = subprocess.run(
                ["ps", "-p", str(pid), "-o", "stat="],
                capture_output=True, text=True,
            )
            if result.returncode != 0 or result.stdout.strip().startswith("Z"):
                return
            self.assertLess(time.monotonic(), deadline, "owned grandchild is still running")
            time.sleep(0.02)

    def test_preserves_command_failure(self):
        result = subprocess.run(self.command(2, "sh", "-c", "exit 42"), capture_output=True)
        self.assertEqual(result.returncode, 42)

    def test_preserves_success_and_output(self):
        result = subprocess.run(
            self.command(2, "sh", "-c", "printf TEST_CODE_output; exit 0"),
            capture_output=True,
        )
        self.assertEqual((result.returncode, result.stdout), (0, b"TEST_CODE_output"))

    def test_deadline_stops_term_ignoring_grandchild(self):
        process, ids = self.group(1)
        _, error = process.communicate(timeout=4)
        self.assertEqual(process.returncode, 2)
        self.assertIn(b"deadline exceeded", error)
        self.assert_stopped(ids["child"])

    def test_supervisor_cancellation_stops_grandchild(self):
        process, ids = self.group(30)
        os.kill(ids["supervisor"], signal.SIGTERM)
        process.communicate(timeout=3)
        self.assertEqual(process.returncode, 128 + signal.SIGTERM)
        self.assert_stopped(ids["child"])


if __name__ == "__main__":
    unittest.main()
