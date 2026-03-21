import json
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import coordinator  # noqa: E402
import run_m3  # noqa: E402


class CoordinatorM3PassCriteriaTests(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.run_dir = Path(self.tmpdir.name) / "m3-pass"
        self.run_dir.mkdir(parents=True, exist_ok=True)
        self.original_repo_state = coordinator.CURRENT_REPO_STATE
        coordinator.CURRENT_REPO_STATE = {
            "head_commit": "current-head",
            "branch": "develop",
            "dirty": False,
            "fingerprint": None,
            "changed_paths": [],
        }
        for rel_path, _ in coordinator.M3_REQUIRED_ARTIFACTS:
            path = self.run_dir / rel_path
            path.parent.mkdir(parents=True, exist_ok=True)
            if path.suffix == ".pcm":
                path.write_bytes(b"\x00\x01")
            elif path.suffix == ".ndjson":
                path.write_text("{\"ok\":true}\n", encoding="utf-8")
            else:
                path.write_text("{\"ok\":true}\n", encoding="utf-8")

    def tearDown(self):
        coordinator.CURRENT_REPO_STATE = self.original_repo_state
        self.tmpdir.cleanup()

    def make_report(self, *, started_at: str, finished_at: str, wall_runtime_sec=None):
        report = {
            "status": "pass",
            "started_at": started_at,
            "finished_at": finished_at,
            "_report_path": str(self.run_dir / "report.json"),
            "_manifest": {
                "milestone": "M3",
                "runtime": {
                    "app_runtime_sec": 1800,
                },
                "repo": {
                    "head_commit": "current-head",
                    "branch": "develop",
                    "dirty": False,
                    "fingerprint": None,
                },
            },
            "_recovery": {
                "recovery_ms": 1026,
                "stable_before": {"ok": True},
                "stable_after": {"ok": True},
                "reconnect_visibility": {"ok": True},
            },
        }
        if wall_runtime_sec is not None:
            report["_recovery"]["wall_runtime_sec"] = wall_runtime_sec
        return report

    def test_short_wall_clock_runtime_does_not_count_as_pass(self):
        report = self.make_report(
            started_at="2026-03-21T23:01:55+08:00",
            finished_at="2026-03-21T23:02:16+08:00",
        )
        self.assertFalse(coordinator.report_counts_as_pass(report, "M3"))

    def test_recovery_wall_runtime_is_used_when_present(self):
        report = self.make_report(
            started_at="2026-03-21T23:01:55+08:00",
            finished_at="2026-03-21T23:02:16+08:00",
            wall_runtime_sec=1802.5,
        )
        self.assertTrue(coordinator.report_counts_as_pass(report, "M3"))

    def test_completion_note_explains_short_debug_run(self):
        report = self.make_report(
            started_at="2026-03-21T23:01:55+08:00",
            finished_at="2026-03-21T23:02:16+08:00",
            wall_runtime_sec=24.0,
        )
        report["_manifest"]["runtime"]["app_runtime_sec"] = 24

        note = coordinator.report_completion_note(report, "M3")

        self.assertIn("24s", note)
        self.assertIn("1800s", note)

    def test_completion_note_preserves_latest_fail_summary(self):
        report = {
            "status": "blocked",
            "summary": "远端工作区同步预检失败：ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
            "_manifest": {"milestone": "M3", "runtime": {"app_runtime_sec": 1800}},
            "_recovery": {},
        }

        note = coordinator.report_completion_note(report, "M3")

        self.assertIn("最近一次 run 为 blocked", note)
        self.assertIn("Operation not permitted", note)

    def test_completion_note_requires_reconnect_visibility(self):
        report = self.make_report(
            started_at="2026-03-21T23:01:55+08:00",
            finished_at="2026-03-22T00:02:16+08:00",
            wall_runtime_sec=1802.5,
        )
        report["_recovery"]["reconnect_visibility"] = {"ok": False}

        note = coordinator.report_completion_note(report, "M3")

        self.assertIn("可见重连/过期提示", note)

    def test_missing_required_artifact_does_not_count_as_pass(self):
        report = self.make_report(
            started_at="2026-03-21T23:01:55+08:00",
            finished_at="2026-03-22T00:02:16+08:00",
            wall_runtime_sec=1802.5,
        )
        (self.run_dir / "server" / "phase2" / "runtime.log").unlink()

        self.assertFalse(coordinator.report_counts_as_pass(report, "M3"))
        note = coordinator.report_completion_note(report, "M3")
        self.assertIn("server/phase2/runtime.log", note)


class CoordinatorRepoFreshnessTests(unittest.TestCase):
    def setUp(self):
        self.original_repo_state = coordinator.CURRENT_REPO_STATE
        coordinator.CURRENT_REPO_STATE = {
            "head_commit": "current-head",
            "branch": "develop",
            "dirty": True,
            "fingerprint": "sha256:new",
            "changed_paths": ["scripts/harness/coordinator.py", "docs/HARNESS.md"],
        }

    def tearDown(self):
        coordinator.CURRENT_REPO_STATE = self.original_repo_state

    def test_missing_repo_snapshot_is_treated_as_stale(self):
        report = {
            "status": "pass",
            "_manifest": {"milestone": "M1"},
            "_recovery": {},
        }

        note = coordinator.report_completion_note(report, "M1")

        self.assertIn("缺少仓库快照", note)
        self.assertIsNone(coordinator.latest_pass_for_milestone([report], "M1"))

    def test_commit_mismatch_is_treated_as_stale(self):
        report = {
            "status": "pass",
            "_manifest": {
                "milestone": "M2",
                "repo": {
                    "head_commit": "old-head",
                    "branch": "develop",
                    "dirty": False,
                    "fingerprint": None,
                },
            },
            "_recovery": {},
        }

        note = coordinator.report_completion_note(report, "M2")

        self.assertIn("old-head", note)
        self.assertIn("current-head", note)
        self.assertIsNone(coordinator.latest_pass_for_milestone([report], "M2"))

    def test_dirty_fingerprint_mismatch_lists_current_changed_paths(self):
        report = {
            "status": "pass",
            "_manifest": {
                "milestone": "M0",
                "repo": {
                    "head_commit": "current-head",
                    "branch": "develop",
                    "dirty": True,
                    "fingerprint": "sha256:old",
                    "changed_paths": ["scripts/harness/run_m0.py"],
                },
            },
            "_recovery": {},
        }

        note = coordinator.report_completion_note(report, "M0")

        self.assertIn("当前工作区改动集已变化", note)
        self.assertIn("scripts/harness/coordinator.py", note)


class CoordinatorSyntheticAttemptStateTests(unittest.TestCase):
    def setUp(self):
        self.original_repo_state = coordinator.CURRENT_REPO_STATE
        coordinator.CURRENT_REPO_STATE = {
            "head_commit": "current-head",
            "branch": "develop",
            "dirty": False,
            "fingerprint": None,
            "changed_paths": [],
        }

    def tearDown(self):
        coordinator.CURRENT_REPO_STATE = self.original_repo_state

    def test_pre_dispatch_blocked_is_attached_to_first_pending_milestone(self):
        state = coordinator.build_state(
            Path("/tmp/netmic-runs"),
            [],
            "M3",
            synthetic_attempt={
                "milestone_id": "M0",
                "run_id": "coordinator-20260322T040000",
                "status": "blocked",
                "summary": "远端工作区同步预检失败：ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
                "finished_at": "2026-03-22T04:00:00+08:00",
                "_report_path": "/tmp/netmic-runs/coordinator-20260322T040000/report.json",
                "_synthetic": True,
            },
        )

        milestones = {item["id"]: item for item in state["milestones"]}
        self.assertEqual(milestones["M0"]["latest_attempt_status"], "blocked")
        self.assertIn("Operation not permitted", milestones["M0"]["note"])
        self.assertEqual(
            milestones["M0"]["latest_attempt_run"],
            "coordinator-20260322T040000",
        )
        self.assertIsNone(milestones["M1"]["latest_attempt_status"])

    def test_pre_dispatch_blocked_moves_to_next_unfinished_milestone_after_m0_pass(self):
        report = {
            "run_id": "m0-pass",
            "status": "pass",
            "finished_at": "2026-03-22T03:00:00+08:00",
            "_report_path": "/tmp/netmic-runs/m0-pass/report.json",
            "_manifest": {
                "milestone": "M0",
                "repo": {
                    "head_commit": "current-head",
                    "branch": "develop",
                    "dirty": False,
                    "fingerprint": None,
                    "changed_paths": [],
                },
            },
            "_recovery": {},
        }

        state = coordinator.build_state(
            Path("/tmp/netmic-runs"),
            [report],
            "M3",
            synthetic_attempt={
                "milestone_id": "M1",
                "run_id": "coordinator-20260322T041000",
                "status": "blocked",
                "summary": "远端工作区同步预检失败：ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
                "finished_at": "2026-03-22T04:10:00+08:00",
                "_report_path": "/tmp/netmic-runs/coordinator-20260322T041000/report.json",
                "_synthetic": True,
            },
        )

        milestones = {item["id"]: item for item in state["milestones"]}
        self.assertEqual(milestones["M0"]["status"], "pass")
        self.assertEqual(milestones["M0"]["latest_attempt_status"], "pass")
        self.assertIsNone(milestones["M0"]["note"])
        self.assertEqual(milestones["M1"]["latest_attempt_status"], "blocked")
        self.assertIn("Operation not permitted", milestones["M1"]["note"])
        self.assertIsNone(milestones["M2"]["latest_attempt_status"])


class RunM3VerdictTests(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.phase2_audio_dump = Path(self.tmpdir.name) / "audio_dump.pcm"
        self.phase2_audio_dump.write_bytes(b"\x00\x01")
        self.ok_window = {
            "ok": True,
            "event_count": 10,
            "max_gap_ms": 1000,
            "unexpected_statuses": [],
        }

    def tearDown(self):
        self.tmpdir.cleanup()

    def test_verdict_rejects_short_observed_runtime(self):
        status, summary = run_m3.evaluate_final_verdict(
            ui_ok=True,
            stable_before_report=self.ok_window,
            stable_after_report=self.ok_window,
            reconnect_visibility={"ok": True},
            app_runtime_sec=1800,
            wall_runtime_sec=21.0,
            recovery_ms=1026,
            phase2_audio_dump=self.phase2_audio_dump,
        )
        self.assertEqual(status, "fail")
        self.assertIn("实际运行时长不达标", summary)

    def test_verdict_accepts_full_runtime_and_audio(self):
        status, summary = run_m3.evaluate_final_verdict(
            ui_ok=True,
            stable_before_report=self.ok_window,
            stable_after_report=self.ok_window,
            reconnect_visibility={"ok": True},
            app_runtime_sec=1800,
            wall_runtime_sec=1804.2,
            recovery_ms=1026,
            phase2_audio_dump=self.phase2_audio_dump,
        )
        self.assertEqual(status, "pass")
        self.assertIn("真实 netmic-ui 长测通过", summary)

    def test_verdict_rejects_missing_reconnect_visibility(self):
        status, summary = run_m3.evaluate_final_verdict(
            ui_ok=True,
            stable_before_report=self.ok_window,
            stable_after_report=self.ok_window,
            reconnect_visibility={
                "ok": False,
                "snapshot_status_note": "等待服务端重连",
                "visible_status_note": "",
                "visible_status_label": "连接中",
            },
            app_runtime_sec=1800,
            wall_runtime_sec=1804.2,
            recovery_ms=1026,
            phase2_audio_dump=self.phase2_audio_dump,
        )
        self.assertEqual(status, "fail")
        self.assertIn("断线提示不可见或不一致", summary)


class RunM3ProcessCleanupTests(unittest.TestCase):
    class FakeProc:
        def __init__(self, poll_result, wait_results):
            self._poll_result = poll_result
            self._wait_results = list(wait_results)
            self.calls = []

        def poll(self):
            self.calls.append("poll")
            return self._poll_result

        def terminate(self):
            self.calls.append("terminate")

        def wait(self, timeout=None):
            self.calls.append(f"wait:{timeout}")
            result = self._wait_results.pop(0)
            if isinstance(result, BaseException):
                raise result
            return result

        def kill(self):
            self.calls.append("kill")

    def test_stop_ui_process_returns_without_action_when_already_exited(self):
        proc = self.FakeProc(0, [])

        forced_kill = run_m3.stop_ui_process(proc)

        self.assertFalse(forced_kill)
        self.assertEqual(proc.calls, ["poll"])

    def test_stop_ui_process_kills_when_graceful_wait_times_out(self):
        proc = self.FakeProc(
            None,
            [
                subprocess.TimeoutExpired(cmd="netmic-ui", timeout=run_m3.PROCESS_STOP_TIMEOUT_SEC),
                0,
            ],
        )

        forced_kill = run_m3.stop_ui_process(proc)

        self.assertTrue(forced_kill)
        self.assertEqual(
            proc.calls,
            [
                "poll",
                "terminate",
                f"wait:{run_m3.PROCESS_STOP_TIMEOUT_SEC}",
                "kill",
                f"wait:{run_m3.PROCESS_STOP_TIMEOUT_SEC}",
            ],
        )


class RunM3ArtifactCollectionTests(unittest.TestCase):
    def test_fetch_remote_artifacts_reports_runtime_fetch_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            server_dir = Path(tmpdir)
            failure = run_m3.run_m0.CommandResult(
                command=["ssh"],
                returncode=255,
                stdout="",
                stderr="ssh: connect to host 192.168.11.1 port 22: Operation not permitted",
            )

            with unittest.mock.patch.object(run_m3.run_m1, "remote_fetch_text", return_value=failure):
                step = run_m3.fetch_remote_artifacts({}, "/remote/run", server_dir, "phase2")

            self.assertEqual(step.status, "blocked")
            self.assertIn("runtime.log", step.summary)
            self.assertTrue((server_dir / "runtime.log").exists())

    def test_fetch_remote_artifacts_reports_audio_dump_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            server_dir = Path(tmpdir)
            runtime = run_m3.run_m0.CommandResult(
                command=["ssh"],
                returncode=0,
                stdout="server log\n",
                stderr="",
            )
            failure = run_m3.run_m0.CommandResult(
                command=["ssh"],
                returncode=1,
                stdout="",
                stderr="missing audio dump",
            )

            with unittest.mock.patch.object(run_m3.run_m1, "remote_fetch_text", return_value=runtime):
                with unittest.mock.patch.object(run_m3.run_m1, "remote_fetch_binary_base64", return_value=failure):
                    step = run_m3.fetch_remote_artifacts({}, "/remote/run", server_dir, "phase2")

            self.assertEqual(step.status, "fail")
            self.assertIn("audio dump", step.summary)
            self.assertEqual((server_dir / "runtime.log").read_text(encoding="utf-8"), "server log\n")
            self.assertEqual((server_dir / "audio_dump.pcm").read_bytes(), b"")


class RunM3RefreshWindowTests(unittest.TestCase):
    def test_refresh_window_rejects_missing_snapshot_status(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 500},
                    },
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2000,
                    "snapshot": {},
                    "visible": {"status_label": "推流中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertFalse(report["ok"])
        self.assertEqual(report["missing_status_count"], 1)

    def test_refresh_window_rejects_visible_label_mismatch(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 500},
                    },
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 1500},
                    },
                    "visible": {"status_label": "连接中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertFalse(report["ok"])
        self.assertEqual(report["unexpected_visible_labels"], ["连接中"])

    def test_refresh_window_accepts_consistent_render_ack_window(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 500},
                    },
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2500,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 2000},
                    },
                    "visible": {"status_label": "推流中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertTrue(report["ok"])
        self.assertEqual(report["expected_visible_label"], "推流中")

    def test_refresh_window_rejects_missing_server_status_timestamp(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 500},
                    },
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 0},
                    },
                    "visible": {"status_label": "推流中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertFalse(report["ok"])
        self.assertEqual(report["missing_status_updated_count"], 1)

    def test_refresh_window_rejects_stale_server_status(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 500},
                    },
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 4201,
                    "snapshot": {
                        "status": "streaming",
                        "runtime": {"server_status_updated_ms": 1000},
                    },
                    "visible": {"status_label": "推流中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertFalse(report["ok"])
        self.assertEqual(report["stale_status_count"], 1)

    def test_reconnect_visibility_requires_matching_visible_note(self):
        report = run_m3.analyze_reconnect_visibility(
            {
                "snapshot": {
                    "status": "connecting",
                    "status_note": "等待服务端重连",
                    "runtime": {"reconnect_attempts": 1},
                },
                "visible": {
                    "status_label": "连接中",
                    "status_note": "",
                },
            }
        )

        self.assertFalse(report["ok"])
        self.assertFalse(report["note_mismatch"])

    def test_reconnect_visibility_accepts_matching_reconnect_note(self):
        report = run_m3.analyze_reconnect_visibility(
            {
                "snapshot": {
                    "status": "connecting",
                    "status_note": "等待服务端重连",
                    "runtime": {"reconnect_attempts": 2},
                },
                "visible": {
                    "status_label": "连接中",
                    "status_note": "等待服务端重连",
                },
            }
        )

        self.assertTrue(report["ok"])
        self.assertTrue(report["note_contains_hint"])

    def test_stale_visibility_requires_expired_hint_for_server_mode(self):
        report = run_m3.analyze_stale_visibility(
            {
                "ts_ms": 6000,
                "snapshot": {
                    "mode": "server",
                    "status": "listening",
                    "runtime": {"server_status_updated_ms": 3000},
                },
                "visible": {
                    "status_label": "监听中",
                    "status_note": "等待客户端连接",
                    "connection_lines": ["模式：Server", "状态：监听中"],
                },
            }
        )

        self.assertFalse(report["ok"])
        self.assertTrue(report["stale_required"])

    def test_stale_visibility_accepts_visible_expired_hint(self):
        report = run_m3.analyze_stale_visibility(
            {
                "ts_ms": 6000,
                "snapshot": {
                    "mode": "server",
                    "status": "listening",
                    "runtime": {"server_status_updated_ms": 3000},
                },
                "visible": {
                    "status_label": "监听中",
                    "status_note": "等待客户端连接",
                    "connection_lines": [
                        "模式：Server",
                        "服务端状态已过期（距最近刷新 3s），请检查状态刷新链路",
                    ],
                },
            }
        )

        self.assertTrue(report["ok"])
        self.assertTrue(report["stale_hint_visible"])


class RunM3VisibleArtifactTests(unittest.TestCase):
    def setUp(self):
        self.tmpdir = tempfile.TemporaryDirectory()
        self.ui_dir = Path(self.tmpdir.name) / "ui"

    def tearDown(self):
        self.tmpdir.cleanup()

    def make_event(self, *, params_lines=None):
        return {
            "ts_ms": 5000,
            "snapshot": {
                "mode": "client",
                "status": "streaming",
                "status_note": "推流中（握手成功）",
                "effective": {
                    "codec": "opus",
                    "sample_rate_hz": 48000,
                    "channels": 1,
                    "chunk_ms": 20,
                    "opus_bitrate_kbps": 48,
                    "jitter_buffer_ms": 100,
                },
                "client_config": {"input_device": "系统默认"},
                "server_config": {"listen_port": 43000},
                "fallbacks": [],
                "runtime": {"server_status_updated_ms": 4500},
            },
            "visible": {
                "status_label": "推流中",
                "status_note": "推流中（握手成功）",
                "primary_action": "停止推流",
                "connection_lines": ["模式：Client", "状态：推流中"],
                "metrics_lines": ["RTT", "4.0 ms"],
                "audio_lines": ["时域波形"],
                "params_lines": params_lines
                if params_lines is not None
                else [
                    "Codec：opus",
                    "采样率：48000 Hz",
                    "声道：1",
                    "Chunk：20 ms",
                    "Opus Bitrate：48",
                    "Buffer：100 ms",
                ],
                "events_lines": ["[INFO] 准备就绪"],
                "config_connection_lines": ["Server IP"],
                "config_audio_lines": ["音频参数"],
                "config_client_lines": ["输入设备", "麦克风权限：granted"],
                "config_server_lines": [],
                "fallback_lines": ["暂无回退记录"],
                "log_filter": "all",
                "log_lines": ["INFO 准备就绪"],
            },
        }

    def test_render_phase_snapshot_writes_real_render_ack_artifacts(self):
        step = run_m3.render_phase_snapshot(self.make_event(), self.ui_dir, "steady")

        self.assertEqual(step.status, "pass")
        status_payload = json.loads((self.ui_dir / "steady" / "visible-status.json").read_text(encoding="utf-8"))
        self.assertEqual(status_payload["status_label"], "推流中")
        self.assertIn("Codec：opus", status_payload["params_lines"])
        refresh_payload = json.loads((self.ui_dir / "steady" / "refresh-check.json").read_text(encoding="utf-8"))
        self.assertTrue(refresh_payload["ok"])
        self.assertEqual(refresh_payload["server_status_updated_ms"], 4500)

    def test_render_phase_snapshot_rejects_incomplete_visible_params(self):
        step = run_m3.render_phase_snapshot(
            self.make_event(params_lines=["Codec：opus"]),
            self.ui_dir,
            "steady",
        )

        self.assertEqual(step.status, "fail")
        self.assertIn("params_lines", step.summary)

    def test_render_phase_snapshot_rejects_missing_stale_hint_for_server_mode(self):
        event = self.make_event()
        event["ts_ms"] = 7000
        event["snapshot"]["mode"] = "server"
        event["snapshot"]["status"] = "listening"
        event["snapshot"]["runtime"]["server_status_updated_ms"] = 4000
        event["visible"]["status_label"] = "监听中"
        event["visible"]["status_note"] = "等待客户端连接"
        event["visible"]["connection_lines"] = ["模式：Server", "状态：监听中"]

        step = run_m3.render_phase_snapshot(event, self.ui_dir, "steady")

        self.assertEqual(step.status, "fail")
        self.assertIn("状态过期提示", step.summary)

    def test_promote_phase_ui_artifacts_copies_standard_top_level_files(self):
        run_m3.render_phase_snapshot(self.make_event(), self.ui_dir, "steady")

        run_m3.promote_phase_ui_artifacts(self.ui_dir, "steady")

        top_level_status = json.loads((self.ui_dir / "visible-status.json").read_text(encoding="utf-8"))
        top_level_refresh = json.loads((self.ui_dir / "refresh-check.json").read_text(encoding="utf-8"))
        self.assertEqual(top_level_status["status_label"], "推流中")
        self.assertTrue(top_level_refresh["ok"])


if __name__ == "__main__":
    unittest.main()
