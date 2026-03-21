#!/usr/bin/env bash
# run_m0.py 远端同步 helper 最小自测：覆盖“有变更则同步”和“Operation not permitted -> blocked”。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

workspace="$tmpdir/workspace"
stub_dir="$tmpdir/stubs"
mkdir -p "$workspace" "$stub_dir"
printf 'netmic\n' >"$workspace/README.md"

cat >"$stub_dir/rsync" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

mode="${RSYNC_TEST_MODE:-changes}"
joined="$*"
printf '%s\n' "$joined" >>"${RSYNC_TEST_LOG:?}"

if [[ "$joined" == *"--dry-run"* ]]; then
  case "$mode" in
    clean)
      exit 0
      ;;
    blocked)
      printf 'ssh: connect to host 192.168.11.1 port 22: Operation not permitted\n' >&2
      exit 255
      ;;
    *)
      printf '>f+++++++++ README.md\n'
      exit 0
      ;;
  esac
fi

case "$mode" in
  changes)
    exit 0
    ;;
  apply-fail)
    printf 'rsync: write failed\n' >&2
    exit 23
    ;;
  *)
    printf 'unexpected apply mode: %s\n' "$mode" >&2
    exit 2
    ;;
esac
EOF
chmod +x "$stub_dir/rsync"

cat >"$stub_dir/sshpass" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-p" ]]; then
  shift 2
fi

exec "$@"
EOF
chmod +x "$stub_dir/sshpass"

run_case() {
  local mode="$1"
  local expect_status="$2"
  local expect_summary="$3"
  local expect_calls="$4"
  local log_path="$tmpdir/$mode.log"

  PATH="$stub_dir:$PATH" \
  RSYNC_TEST_MODE="$mode" \
  RSYNC_TEST_LOG="$log_path" \
  PYTHONPATH="$ROOT/scripts/harness" \
  python3 - "$workspace" "$expect_status" "$expect_summary" "$expect_calls" "$log_path" "$mode" <<'PY'
import pathlib
import sys

import run_m0

workspace = pathlib.Path(sys.argv[1])
expect_status = sys.argv[2]
expect_summary = sys.argv[3]
expect_calls = int(sys.argv[4])
log_path = pathlib.Path(sys.argv[5])
mode = sys.argv[6]

env = {
    "NETMIC_HARNESS_COORDINATOR_ROOT": str(workspace),
    "NETMIC_HARNESS_LINUX_HOST": "192.168.11.1",
    "NETMIC_HARNESS_LINUX_PORT": "22",
    "NETMIC_HARNESS_LINUX_USER": "arc",
    "NETMIC_HARNESS_LINUX_ROOT": "/home/arc/code/NetMic",
    "NETMIC_HARNESS_LINUX_PASSWORD": "top-secret",
}

status, summary, results = run_m0.sync_remote_workspace(env)
assert status == expect_status, (status, summary)
assert expect_summary in summary, summary
assert len(results) == expect_calls, len(results)

commands = log_path.read_text(encoding="utf-8").splitlines()
assert len(commands) == expect_calls, commands
assert all("--exclude .harness/runs/" in command for command in commands), commands

server_dir = workspace / "artifacts" / mode
step = run_m0.run_remote_sync_step(env, server_dir)
assert step.status == expect_status, step
state = run_m0.json.loads((server_dir / "remote-sync.json").read_text(encoding="utf-8"))
for item in state["commands"]:
    command = item["command"]
    assert "top-secret" not in command, command
    assert "<redacted>" in command, command
PY
}

run_case changes pass "已将本地工作区同步到远端" 2
run_case clean pass "远端工作区已与本地同步" 1
run_case blocked blocked "Operation not permitted" 1
run_case apply-fail fail "rsync: write failed" 2

echo "[ok] sync_remote helper 最小测试通过"
