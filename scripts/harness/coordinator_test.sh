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
