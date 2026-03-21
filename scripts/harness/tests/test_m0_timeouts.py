import sys
import unittest
from pathlib import Path


HARNESS_DIR = Path(__file__).resolve().parents[1]
if str(HARNESS_DIR) not in sys.path:
    sys.path.insert(0, str(HARNESS_DIR))

import run_m0  # noqa: E402


class RunM0TimeoutTests(unittest.TestCase):
    def test_remote_timeout_for_runtime_extends_known_long_steps(self):
        env = {"NETMIC_HARNESS_REMOTE_TIMEOUT_SEC": "120"}
        self.assertEqual(run_m0.remote_timeout_for_runtime(env, 300), 330)

    def test_remote_timeout_for_runtime_respects_larger_env_timeout(self):
        env = {"NETMIC_HARNESS_REMOTE_TIMEOUT_SEC": "500"}
        self.assertEqual(run_m0.remote_timeout_for_runtime(env, 300), 500)


if __name__ == "__main__":
    unittest.main()
