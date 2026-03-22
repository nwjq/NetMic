import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402


def result(command, stdout="", stderr="", returncode=0):
    return run_m0.CommandResult(
        command=command,
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
    )


class GitSyncPathTests(unittest.TestCase):
    def make_env(self, root: str) -> dict[str, str]:
        return {
            "NETMIC_HARNESS_COORDINATOR_ROOT": root,
            "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
            "NETMIC_HARNESS_LINUX_PORT": "22",
            "NETMIC_HARNESS_LINUX_USER": "arc",
            "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
            "NETMIC_HARNESS_LINUX_PASSWORD": "top-secret",
        }

    def test_same_origin_clean_repos_use_git_push_pull(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            Path(tmpdir, "README.md").write_text("NetMic\n", encoding="utf-8")
            env = self.make_env(tmpdir)

            def fake_run_command(command, timeout_sec=None, cwd=None):
                mapping = {
                    ("git", "rev-parse", "--show-toplevel"): result(command, f"{tmpdir}\n"),
                    ("git", "branch", "--show-current"): result(command, "develop\n"),
                    ("git", "rev-parse", "HEAD"): result(command, "local-head\n"),
                    ("git", "remote", "get-url", "origin"): result(command, "https://example.com/NetMic.git\n"),
                    ("git", "status", "--porcelain=v1", "--untracked-files=all", "--ignored=no"): result(
                        command,
                        " M AGENTS.md\n",
                    ),
                    ("git", "push", "origin", "develop"): result(command, "Everything up-to-date\n"),
                }
                key = tuple(command)
                if key not in mapping:
                    raise AssertionError(f"unexpected local command: {command}")
                return mapping[key]

            def fake_run_remote(remote_env, script, timeout_sec=None):
                if "__NETMIC_GIT_STATE__" in script:
                    return result(
                        ["ssh", script],
                        "\n".join(
                            [
                                "__NETMIC_GIT_STATE__",
                                "branch=develop",
                                "head=remote-head",
                                "origin=https://example.com/NetMic.git",
                                "__NETMIC_GIT_STATUS__",
                                "",
                            ]
                        ),
                    )
                if script == "git pull --ff-only origin develop":
                    return result(["ssh", script], "Updating remote-head..local-head\n")
                raise AssertionError(f"unexpected remote script: {script}")

            with mock.patch.object(run_m0, "run_command", side_effect=fake_run_command):
                with mock.patch.object(run_m0, "run_remote", side_effect=fake_run_remote):
                    status, summary, commands = run_m0.sync_remote_workspace(env)

        self.assertEqual(status, "pass")
        self.assertIn("git push/pull", summary)
        self.assertEqual(
            [run_m0.sync_command_title(item, i) for i, item in enumerate(commands)],
            ["git-state-probe", "git-push", "git-pull"],
        )

    def test_same_origin_remote_dirty_is_fail(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            Path(tmpdir, "README.md").write_text("NetMic\n", encoding="utf-8")
            env = self.make_env(tmpdir)

            def fake_run_command(command, timeout_sec=None, cwd=None):
                mapping = {
                    ("git", "rev-parse", "--show-toplevel"): result(command, f"{tmpdir}\n"),
                    ("git", "branch", "--show-current"): result(command, "develop\n"),
                    ("git", "rev-parse", "HEAD"): result(command, "local-head\n"),
                    ("git", "remote", "get-url", "origin"): result(command, "https://example.com/NetMic.git\n"),
                    ("git", "status", "--porcelain=v1", "--untracked-files=all", "--ignored=no"): result(command, ""),
                }
                key = tuple(command)
                if key not in mapping:
                    raise AssertionError(f"unexpected local command: {command}")
                return mapping[key]

            def fake_run_remote(remote_env, script, timeout_sec=None):
                if "__NETMIC_GIT_STATE__" in script:
                    return result(
                        ["ssh", script],
                        "\n".join(
                            [
                                "__NETMIC_GIT_STATE__",
                                "branch=develop",
                                "head=remote-head",
                                "origin=https://example.com/NetMic.git",
                                "__NETMIC_GIT_STATUS__",
                                " M apps/netmic-ui/src-tauri/src/main.rs",
                                "?? scripts/harness/run_m3.py",
                            ]
                        ),
                    )
                raise AssertionError(f"unexpected remote script: {script}")

            with mock.patch.object(run_m0, "run_command", side_effect=fake_run_command):
                with mock.patch.object(run_m0, "run_remote", side_effect=fake_run_remote):
                    status, summary, commands = run_m0.sync_remote_workspace(env)

        self.assertEqual(status, "fail")
        self.assertIn("远端仓库存在未提交改动", summary)
        self.assertIn("apps/netmic-ui/src-tauri/src/main.rs", summary)
        self.assertEqual(
            [run_m0.sync_command_title(item, i) for i, item in enumerate(commands)],
            ["git-state-probe"],
        )


if __name__ == "__main__":
    unittest.main()
