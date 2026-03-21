import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402
import run_m1  # noqa: E402
import run_m3  # noqa: E402


class CommandFailureSummaryTests(unittest.TestCase):
    def test_build_command_failure_summary_keeps_first_real_error(self):
        output = "\n".join(
            [
                "sending incremental file list",
                "",
                "ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
            ]
        )

        summary = run_m0.build_command_failure_summary("远端工作区同步预检失败", output)

        self.assertEqual(
            summary,
            "远端工作区同步预检失败：ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
        )


class RunM3RemoteBootstrapFailureTests(unittest.TestCase):
    def test_remote_start_permission_error_is_reported_as_blocked(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir_path = Path(tmpdir)
            artifact_root = tmpdir_path / "runs"
            hosts_env = tmpdir_path / "hosts.env"
            hosts_env.write_text(
                "\n".join(
                    [
                        "NETMIC_HARNESS_COORDINATOR_ROOT=/Users/arc/code/NetMic",
                        "NETMIC_HARNESS_MAC_ROOT=/Users/arc/code/NetMic",
                        "NETMIC_HARNESS_LINUX_HOST=192.168.11.1",
                        "NETMIC_HARNESS_LINUX_PORT=22",
                        "NETMIC_HARNESS_LINUX_USER=arc",
                        "NETMIC_HARNESS_LINUX_ROOT=/home/arc/code/NetMic",
                        "NETMIC_HARNESS_LINUX_PASSWORD=top-secret",
                        "NETMIC_HARNESS_SERVER_HOST=192.168.11.1",
                        "NETMIC_HARNESS_SERVER_PORT=43000",
                        f"NETMIC_HARNESS_ARTIFACT_DIR={artifact_root}",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            remote_error = run_m0.CommandResult(
                command=["ssh"],
                returncode=255,
                stdout="",
                stderr="ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
            )

            with mock.patch.object(sys, "argv", ["run_m3.py", "--hosts-env", str(hosts_env)]):
                with mock.patch.object(run_m0, "require_local_tools", return_value=("pass", "本地依赖检查通过")):
                    with mock.patch.object(run_m0.shutil, "which", return_value="/usr/bin/cargo"):
                        with mock.patch.object(
                            run_m0,
                            "run_remote_sync_step",
                            return_value=run_m0.StepResult("sync-remote", "pass", "远端工作区已与本地同步"),
                        ):
                            with mock.patch.object(run_m1, "remote_start_server", return_value=remote_error):
                                exit_code = run_m3.main()

            self.assertEqual(exit_code, 2)

            run_dirs = sorted(artifact_root.glob("m3-*/report.json"))
            self.assertEqual(len(run_dirs), 1)
            report = json.loads(run_dirs[0].read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "blocked")
            self.assertIn("Operation not permitted", report["summary"])
            self.assertIn("phase1 启动失败", report["summary"])


if __name__ == "__main__":
    unittest.main()
