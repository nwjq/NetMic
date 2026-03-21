#!/usr/bin/env bash
# coordinator.py 最小自测：已有 M0/M1 pass 时，必须继续推进到 M2，并在缺少现场配置时明确 blocked。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/harness/coordinator.py"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

artifact_root="$tmpdir/runs"
mkdir -p "$artifact_root/m0-pass" "$artifact_root/m1-pass"

cat >"$artifact_root/m0-pass/manifest.json" <<'EOF'
{
  "run_id": "m0-pass",
  "milestone": "M0"
}
EOF

cat >"$artifact_root/m0-pass/report.json" <<'EOF'
{
  "run_id": "m0-pass",
  "status": "pass",
  "summary": "M0 ok",
  "finished_at": "2026-03-21T21:59:25+08:00"
}
EOF

cat >"$artifact_root/m1-pass/manifest.json" <<'EOF'
{
  "run_id": "m1-pass",
  "milestone": "M1"
}
EOF

cat >"$artifact_root/m1-pass/report.json" <<'EOF'
{
  "run_id": "m1-pass",
  "status": "pass",
  "summary": "M1 ok",
  "finished_at": "2026-03-21T22:09:25+08:00"
}
EOF

cat >"$tmpdir/hosts.env" <<EOF
NETMIC_HARNESS_ARTIFACT_DIR=$artifact_root
EOF

set +e
output="$(python3 "$TARGET" --hosts-env "$tmpdir/hosts.env" --until M3 2>&1)"
status=$?
set -e

if [[ "$status" -ne 2 ]]; then
  echo "期望 coordinator 因 M2 缺少现场配置而 blocked，实际退出码=$status" >&2
  echo "$output" >&2
  exit 1
fi

case "$output" in
  *"缺少现场配置"*) ;;
  *)
    echo "未输出 M2 现场配置缺失信息" >&2
    echo "$output" >&2
    exit 1
    ;;
esac

echo "[ok] coordinator.py 最小测试通过"

mkdir -p "$artifact_root/m2-pass" "$artifact_root/m3-short-pass"

cat >"$artifact_root/m2-pass/manifest.json" <<'EOF'
{
  "run_id": "m2-pass",
  "milestone": "M2"
}
EOF

cat >"$artifact_root/m2-pass/report.json" <<'EOF'
{
  "run_id": "m2-pass",
  "status": "pass",
  "summary": "M2 ok",
  "finished_at": "2026-03-21T22:54:47+08:00"
}
EOF

cat >"$artifact_root/m3-short-pass/manifest.json" <<'EOF'
{
  "run_id": "m3-short-pass",
  "milestone": "M3",
  "runtime": {
    "app_runtime_sec": 24
  }
}
EOF

cat >"$artifact_root/m3-short-pass/recovery.json" <<'EOF'
{
  "recovery_ms": 1026,
  "stable_before": {
    "ok": true
  },
  "stable_after": {
    "ok": true
  }
}
EOF

cat >"$artifact_root/m3-short-pass/report.json" <<'EOF'
{
  "run_id": "m3-short-pass",
  "status": "pass",
  "summary": "M3 short ok",
  "finished_at": "2026-03-21T23:02:16+08:00",
  "recovery_ms": 1026
}
EOF

set +e
output="$(python3 "$TARGET" --hosts-env "$tmpdir/hosts.env" --until M3 2>&1)"
status=$?
set -e

if [[ "$status" -ne 2 ]]; then
  echo "期望 coordinator 忽略短时 M3 旧产物并继续推进，实际退出码=$status" >&2
  echo "$output" >&2
  exit 1
fi

case "$output" in
  *"缺少现场配置"*) ;;
  *)
    echo "短时 M3 旧产物被误判为完成，未继续触发 M3 runner" >&2
    echo "$output" >&2
    exit 1
    ;;
esac

echo "[ok] coordinator.py M3 短时旧产物回归测试通过"

sync_tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir" "$sync_tmpdir"' EXIT

sync_artifact_root="$sync_tmpdir/runs"
sync_stub_dir="$sync_tmpdir/stubs"
mkdir -p "$sync_artifact_root" "$sync_stub_dir"

cat >"$sync_stub_dir/rsync" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'ssh: connect to host 192.168.11.1 port 22: Operation not permitted\n' >&2
exit 255
EOF
chmod +x "$sync_stub_dir/rsync"

cat >"$sync_stub_dir/sshpass" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "-p" ]]; then
  shift 2
fi
exec "$@"
EOF
chmod +x "$sync_stub_dir/sshpass"

cat >"$sync_tmpdir/hosts.env" <<EOF
NETMIC_HARNESS_COORDINATOR_ROOT=$ROOT
NETMIC_HARNESS_MAC_ROOT=$ROOT
NETMIC_HARNESS_LINUX_HOST=192.168.11.1
NETMIC_HARNESS_LINUX_PORT=22
NETMIC_HARNESS_LINUX_USER=arc
NETMIC_HARNESS_LINUX_ROOT=/home/arc/code/NetMic
NETMIC_HARNESS_LINUX_PASSWORD=top-secret
NETMIC_HARNESS_SERVER_HOST=192.168.11.1
NETMIC_HARNESS_SERVER_PORT=43000
NETMIC_HARNESS_ARTIFACT_DIR=$sync_artifact_root
EOF

set +e
output="$(PATH="$sync_stub_dir:$PATH" python3 "$TARGET" --hosts-env "$sync_tmpdir/hosts.env" --until M3 2>&1)"
status=$?
set -e

if [[ "$status" -ne 2 ]]; then
  echo "期望 coordinator 因远端同步预检失败而 blocked，实际退出码=$status" >&2
  echo "$output" >&2
  exit 1
fi

case "$output" in
  *"远端工作区同步预检失败：ssh: connect to host 192.168.11.1 port 22: Operation not permitted"*) ;;
  *)
    echo "未输出远端同步预检的底层 SSH 错误" >&2
    echo "$output" >&2
    exit 1
    ;;
esac

sync_state="$(find "$sync_artifact_root" -path '*/sync/remote-sync.json' | head -n 1)"
sync_log="$(find "$sync_artifact_root" -path '*/sync/remote-sync.log' | head -n 1)"
if [[ -z "$sync_state" || -z "$sync_log" ]]; then
  echo "缺少 coordinator 远端同步产物" >&2
  exit 1
fi

python3 - "$sync_state" "$sync_log" <<'PY'
import json
import sys

state_path, log_path = sys.argv[1], sys.argv[2]
state = json.load(open(state_path, "r", encoding="utf-8"))
log_text = open(log_path, "r", encoding="utf-8").read()

assert state["status"] == "blocked", state
assert "Operation not permitted" in state["summary"], state
assert state["commands"][0]["returncode"] == 255, state
assert "top-secret" not in json.dumps(state, ensure_ascii=False), state
assert "Operation not permitted" in log_text, log_text
PY

echo "[ok] coordinator.py 远端同步阻塞回归测试通过"
