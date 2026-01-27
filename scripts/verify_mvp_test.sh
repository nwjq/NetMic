#!/usr/bin/env bash
# verify_mvp.py 的最小自测：覆盖 fail/blocked/缺失脚本三类判定。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="$ROOT/scripts/verify_mvp.py"

if [[ ! -x "$TARGET" ]]; then
  echo "目标脚本不存在或不可执行：$TARGET" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

run_case() {
  local name="$1"
  local gates_file="$2"
  local expect_exit="$3"

  local state_dir="$tmpdir/state_$name"
  mkdir -p "$state_dir"

  local out_file="$tmpdir/out_$name.txt"
  set +e
  "$TARGET" --gates "$gates_file" --state-dir "$state_dir" --hub-url "http://127.0.0.1:7788" >"$out_file" 2>&1
  local code=$?
  set -e

  if [[ "$code" -ne "$expect_exit" ]]; then
    echo "[$name] 退出码不符合预期：got=$code expect=$expect_exit" >&2
    cat "$out_file" >&2
    exit 1
  fi

  python3 - "$state_dir/verify_status.json" "$name" <<'PY'
import json
import sys

path, name = sys.argv[1:3]
data = json.load(open(path, "r", encoding="utf-8"))
overall = data.get("overall", {})
status = overall.get("overall_status")
active = overall.get("active_gate_id")

if name == "fail":
    assert status == "fail", status
    assert active == "G1", active
    assert overall.get("failure_signature"), "missing failure_signature"
elif name == "blocked":
    assert status == "blocked", status
    assert active == "G1", active
elif name == "missing_script":
    assert status == "fail", status
    assert active == "G1", active
else:
    raise SystemExit(f"unknown case: {name}")
PY

  echo "[ok] $name"
}

# case 1: fail（业务失败）
gates_fail="$tmpdir/gates_fail.yaml"
cat >"$gates_fail" <<'JSON'
{
  "verify_defaults": {"success_exit_codes": [0], "blocked_exit_codes": [126, 127], "timeout_sec": 30},
  "blocked_patterns_default": [],
  "gates": [
    {"id": "G0", "title": "pass", "checks": [{"id": "g0-pass", "run": "true"}]},
    {"id": "G1", "title": "fail", "checks": [{"id": "g1-fail", "run": "bash -lc 'echo intentional-fail >&2; exit 3'"}]}
  ]
}
JSON
run_case "fail" "$gates_fail" 1

# case 2: blocked（环境阻塞）
gates_blocked="$tmpdir/gates_blocked.yaml"
cat >"$gates_blocked" <<'JSON'
{
  "verify_defaults": {"success_exit_codes": [0], "blocked_exit_codes": [126, 127], "timeout_sec": 30},
  "blocked_patterns_default": ["command not found"],
  "gates": [
    {"id": "G0", "title": "pass", "checks": [{"id": "g0-pass", "run": "true"}]},
    {"id": "G1", "title": "blocked", "checks": [{"id": "g1-blocked", "run": "missing_command_netmic"}]}
  ]
}
JSON
run_case "blocked" "$gates_blocked" 2

# case 3: missing repo script（应判定为 fail，而不是 blocked）
gates_missing_script="$tmpdir/gates_missing_script.yaml"
cat >"$gates_missing_script" <<'JSON'
{
  "verify_defaults": {"success_exit_codes": [0], "blocked_exit_codes": [126, 127], "timeout_sec": 30},
  "blocked_patterns_default": [],
  "gates": [
    {"id": "G0", "title": "pass", "checks": [{"id": "g0-pass", "run": "true"}]},
    {"id": "G1", "title": "missing-script", "checks": [{"id": "g1-missing", "run": "scripts/does_not_exist.sh"}]}
  ]
}
JSON
run_case "missing_script" "$gates_missing_script" 1

echo "[ok] verify_mvp.py 最小自测通过"
