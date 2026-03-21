import sys
import tempfile
import unittest
from pathlib import Path


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import coordinator  # noqa: E402
import run_m3  # noqa: E402


class CoordinatorM3PassCriteriaTests(unittest.TestCase):
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

    def make_report(self, *, started_at: str, finished_at: str, wall_runtime_sec=None):
        report = {
            "status": "pass",
            "started_at": started_at,
            "finished_at": finished_at,
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
            app_runtime_sec=1800,
            wall_runtime_sec=1804.2,
            recovery_ms=1026,
            phase2_audio_dump=self.phase2_audio_dump,
        )
        self.assertEqual(status, "pass")
        self.assertIn("真实 netmic-ui 长测通过", summary)


class RunM3RefreshWindowTests(unittest.TestCase):
    def test_refresh_window_rejects_missing_snapshot_status(self):
        report = run_m3.analyze_refresh_window(
            [
                {
                    "ts_ms": 1000,
                    "snapshot": {"status": "streaming"},
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
                    "snapshot": {"status": "streaming"},
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2000,
                    "snapshot": {"status": "streaming"},
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
                    "snapshot": {"status": "streaming"},
                    "visible": {"status_label": "推流中"},
                },
                {
                    "ts_ms": 2500,
                    "snapshot": {"status": "streaming"},
                    "visible": {"status_label": "推流中"},
                },
            ],
            0,
            1,
            "streaming",
        )

        self.assertTrue(report["ok"])
        self.assertEqual(report["expected_visible_label"], "推流中")


if __name__ == "__main__":
    unittest.main()
