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

    def test_phase2_server_is_stopped_when_recovery_window_fails(self):
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

            start_result = run_m0.CommandResult(
                command=["ssh"],
                returncode=0,
                stdout="123\n",
                stderr="",
            )
            status_result = run_m0.CommandResult(
                command=["ssh"],
                returncode=0,
                stdout=json.dumps({"state": "listening", "virtual_mic_ready": True}),
                stderr="",
            )

            class FakeProc:
                def poll(self):
                    return None

                def terminate(self):
                    return None

                def wait(self, timeout=None):
                    return 0

                def kill(self):
                    return None

            stop_calls = []

            def record_stop(env, remote_dir):
                stop_calls.append(remote_dir)
                return run_m0.CommandResult(command=["ssh"], returncode=0, stdout="", stderr="")

            event_sequence = [
                (
                    0,
                    {
                        "ts_ms": 1_000,
                        "snapshot": {"status": "streaming", "runtime": {"reconnect_attempts": 0}},
                        "visible": {"status_label": "推流中"},
                    },
                ),
                (
                    1,
                    {
                        "ts_ms": 3_000,
                        "snapshot": {"status": "connecting", "runtime": {"reconnect_attempts": 1}},
                        "visible": {"status_label": "连接中"},
                    },
                ),
                (None, None),
            ]

            with mock.patch.object(sys, "argv", ["run_m3.py", "--hosts-env", str(hosts_env)]):
                with mock.patch.object(
                    run_m0,
                    "collect_repo_state",
                    return_value={
                        "available": True,
                        "head_commit": "test-head",
                        "branch": "develop",
                        "dirty": False,
                        "fingerprint": None,
                        "changed_paths": [],
                    },
                ):
                    with mock.patch.object(run_m0, "require_local_tools", return_value=("pass", "本地依赖检查通过")):
                        with mock.patch.object(run_m3.shutil, "which", return_value="/usr/bin/cargo"):
                            with mock.patch.object(
                                run_m0,
                                "run_remote_sync_step",
                                return_value=run_m0.StepResult("sync-remote", "pass", "远端工作区已与本地同步"),
                            ):
                                with mock.patch.object(run_m1, "remote_start_server", side_effect=[start_result, start_result]):
                                    with mock.patch.object(run_m1, "remote_status", return_value=status_result):
                                        with mock.patch.object(run_m1, "remote_stop_server", side_effect=record_stop):
                                            with mock.patch.object(run_m3.subprocess, "Popen", return_value=FakeProc()):
                                                with mock.patch.object(run_m3, "wait_for_event", side_effect=event_sequence):
                                                    with mock.patch.object(
                                                        run_m3,
                                                        "wait_for_event_span",
                                                        return_value=(0, {"ts_ms": 2_000, "snapshot": {"status": "streaming"}}),
                                                    ):
                                                        with mock.patch.object(run_m3, "fetch_remote_artifacts", return_value=None):
                                                            with mock.patch.object(run_m0, "append_section", return_value=None):
                                                                exit_code = run_m3.main()

            self.assertEqual(exit_code, 1)
            self.assertTrue(any(path.endswith("/server-phase2") for path in stop_calls), stop_calls)

            run_dirs = sorted(artifact_root.glob("m3-*/report.json"))
            self.assertEqual(len(run_dirs), 1)
            report = json.loads(run_dirs[0].read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "fail")
            self.assertIn("恢复窗口", report["summary"])

            recovery_path = run_dirs[0].with_name("recovery.json")
            self.assertTrue(recovery_path.exists())
            recovery = json.loads(recovery_path.read_text(encoding="utf-8"))
            self.assertEqual(recovery["phase"], "disconnect-recover")
            self.assertEqual(recovery["status"], "fail")
            self.assertIn("恢复窗口", recovery["summary"])
            self.assertEqual(recovery["render_event_count"], 0)
            self.assertEqual(recovery["streaming_event"]["ts_ms"], 1000)
            self.assertEqual(recovery["reconnecting_event"]["ts_ms"], 3000)
            self.assertIsNone(recovery["recovered_event"])


if __name__ == "__main__":
    unittest.main()
