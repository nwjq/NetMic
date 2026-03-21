import sys
import unittest
from pathlib import Path
from unittest import mock


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402


def command_result(stdout: str) -> run_m0.CommandResult:
    return run_m0.CommandResult(
        command=["git"],
        returncode=0,
        stdout=stdout,
        stderr="",
    )


class RepoStateFilterTests(unittest.TestCase):
    def test_collect_repo_state_ignores_agents_md_noise(self):
        with mock.patch.object(
            run_m0,
            "run_command",
            side_effect=[
                command_result("current-head\n"),
                command_result("develop\n"),
                command_result(" M AGENTS.md\n"),
            ],
        ):
            with mock.patch.object(run_m0, "_hash_repo_path", return_value="sha256:test"):
                repo_state = run_m0.collect_repo_state()

        self.assertTrue(repo_state["available"])
        self.assertFalse(repo_state["dirty"])
        self.assertIsNone(repo_state["fingerprint"])
        self.assertEqual(repo_state["changed_paths"], [])

    def test_collect_repo_state_keeps_harness_changes(self):
        with mock.patch.object(
            run_m0,
            "run_command",
            side_effect=[
                command_result("current-head\n"),
                command_result("develop\n"),
                command_result(" M scripts/harness/run_m0.py\n"),
            ],
        ):
            with mock.patch.object(run_m0, "_hash_repo_path", return_value="sha256:test"):
                repo_state = run_m0.collect_repo_state()

        self.assertTrue(repo_state["available"])
        self.assertTrue(repo_state["dirty"])
        self.assertEqual(repo_state["changed_paths"], ["scripts/harness/run_m0.py"])
        self.assertTrue(str(repo_state["fingerprint"]).startswith("sha256:"))


if __name__ == "__main__":
    unittest.main()
