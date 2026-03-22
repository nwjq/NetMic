import sys
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402


class RunRemoteWrapperTests(unittest.TestCase):
    def setUp(self) -> None:
        self.env = {
            "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
            "NETMIC_HARNESS_LINUX_USER": "arc",
            "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
        }

    def test_run_remote_uses_marker_status_and_strips_marker_line(self):
        raw = run_m0.CommandResult(
            command=["ssh"],
            returncode=1,
            stdout="payload line\n__NETMIC_REMOTE_EXIT__=0\n",
            stderr="",
        )
        with mock.patch.object(run_m0, "run_command", return_value=raw):
            with mock.patch.object(run_m0, "build_ssh_base", return_value=["ssh"]):
                result = run_m0.run_remote(self.env, "echo ok")

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "payload line\n")

    def test_run_remote_preserves_ssh_transport_failure_without_marker(self):
        raw = run_m0.CommandResult(
            command=["ssh"],
            returncode=255,
            stdout="",
            stderr="Permission denied",
        )
        with mock.patch.object(run_m0, "run_command", return_value=raw):
            with mock.patch.object(run_m0, "build_ssh_base", return_value=["ssh"]):
                result = run_m0.run_remote(self.env, "echo ok")

        self.assertEqual(result.returncode, 255)
        self.assertEqual(result.stderr, "Permission denied")


if __name__ == "__main__":
    unittest.main()
