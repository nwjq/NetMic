import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402


class SyncRemoteGitTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.addCleanup(self.tempdir.cleanup)
        self.local_root = Path(self.tempdir.name)
        self.env = {
            "NETMIC_HARNESS_COORDINATOR_ROOT": str(self.local_root),
            "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
            "NETMIC_HARNESS_LINUX_USER": "arc",
            "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
        }

    def test_sync_remote_workspace_skips_remote_git_probe_when_local_needs_rsync_fallback(self):
        with mock.patch.object(
            run_m0,
            "inspect_local_git_sync_state",
            return_value={"available": False, "fallback_allowed": True},
        ), mock.patch.object(
            run_m0,
            "sync_remote_workspace_via_rsync",
            return_value=("pass", "rsync fallback", []),
        ) as rsync_sync, mock.patch.object(
            run_m0,
            "inspect_remote_git_sync_state",
        ) as remote_probe:
            status, summary, results = run_m0.sync_remote_workspace(self.env)

        self.assertEqual(("pass", "rsync fallback", []), (status, summary, results))
        rsync_sync.assert_called_once()
        remote_probe.assert_not_called()

    def test_sync_remote_workspace_fails_when_remote_same_origin_is_dirty(self):
        remote_probe = run_m0.CommandResult(["ssh", "git-status"], 0, "", "")
        with mock.patch.object(
            run_m0,
            "inspect_local_git_sync_state",
            return_value={
                "available": True,
                "origin": "https://github.com/nwjq/NetMic.git",
                "branch": "develop",
                "head_commit": "abc",
                "dirty_paths": [],
            },
        ), mock.patch.object(
            run_m0,
            "inspect_remote_git_sync_state",
            return_value=(
                {
                    "available": True,
                    "origin": "https://github.com/nwjq/NetMic.git",
                    "branch": "develop",
                    "head_commit": "def",
                    "dirty_paths": ["apps/netmic-ui/ui/app.js"],
                },
                remote_probe,
            ),
        ):
            status, summary, results = run_m0.sync_remote_workspace(self.env)

        self.assertEqual("fail", status)
        self.assertIn("不能直接 git pull --ff-only", summary)
        self.assertIn("apps/netmic-ui/ui/app.js", summary)
        self.assertEqual([remote_probe], results)

    def test_sync_remote_workspace_via_git_runs_push_then_pull(self):
        remote_probe = run_m0.CommandResult(["ssh", "git-status"], 0, "", "")
        push_result = run_m0.CommandResult(["git", "push", "origin", "develop"], 0, "", "")
        pull_result = run_m0.CommandResult(["ssh", "git-pull"], 0, "", "")
        with mock.patch.object(run_m0, "run_command", return_value=push_result) as run_command, mock.patch.object(
            run_m0,
            "run_remote",
            return_value=pull_result,
        ) as run_remote:
            status, summary, results = run_m0.sync_remote_workspace_via_git(
                self.env,
                self.local_root,
                {
                    "branch": "develop",
                    "head_commit": "abc",
                    "dirty_paths": [],
                },
                {
                    "branch": "develop",
                    "head_commit": "def",
                    "dirty_paths": [],
                },
                remote_probe,
            )

        self.assertEqual("pass", status)
        self.assertEqual("已通过 git push/pull 同步远端仓库", summary)
        self.assertEqual([remote_probe, push_result, pull_result], results)
        run_command.assert_called_once_with(
            ["git", "push", "origin", "develop"],
            timeout_sec=run_m0.remote_timeout_sec(self.env),
            cwd=self.local_root,
        )
        run_remote.assert_called_once()


if __name__ == "__main__":
    unittest.main()
