import sys
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402
import run_m1  # noqa: E402


class RemoteServerCleanupTests(unittest.TestCase):
    def setUp(self) -> None:
        self.env = {
            "NETMIC_HARNESS_SERVER_PORT": "43000",
        }

    def test_remote_start_server_cleans_port_and_checks_liveness(self):
        with mock.patch.object(
            run_m0,
            "run_remote",
            return_value=run_m0.CommandResult([], 0, "", ""),
        ) as run_remote:
            run_m1.remote_start_server(self.env, "/tmp/netmic-phase", 43000)

        script = run_remote.call_args.args[1]
        self.assertIn("cleanup_named_processes()", script)
        self.assertIn("cleanup_port_pids()", script)
        self.assertIn("ensure_port_released", script)
        self.assertIn("port_is_busy()", script)
        self.assertIn("UDP port 43000 remains busy after cleanup", script)
        self.assertIn("grep ':43000'", script)
        self.assertIn("lsof -t -iUDP:43000", script)
        self.assertIn("if [ \"$pid\" = \"$$\" ] || [ \"$pid\" = \"$PPID\" ]", script)
        self.assertIn("if ! kill -0 \"$pid\"", script)
        self.assertIn("failed to bind UDP socket", script)
        self.assertIn("did not bind UDP port 43000 in time", script)
        self.assertIn("cat /tmp/netmic-phase/runtime.log", script)

    def test_remote_stop_server_cleans_pid_file_and_udp_port(self):
        with mock.patch.object(
            run_m0,
            "run_remote",
            return_value=run_m0.CommandResult([], 0, "", ""),
        ) as run_remote:
            run_m1.remote_stop_server(self.env, "/tmp/netmic-phase")

        script = run_remote.call_args.args[1]
        self.assertIn("cleanup_named_processes()", script)
        self.assertIn("cleanup_port_pids()", script)
        self.assertIn("grep ':43000'", script)
        self.assertIn("lsof -t -iUDP:43000", script)
        self.assertIn("if [ \"$pid\" = \"$$\" ] || [ \"$pid\" = \"$PPID\" ]", script)
        self.assertIn("rm -f /tmp/netmic-phase/server.pid", script)


if __name__ == "__main__":
    unittest.main()
