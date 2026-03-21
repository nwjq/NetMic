#!/usr/bin/env bash
# install_launchd_agent.sh 最小自测：模板本身可 lint，渲染结果带入 interval/路径后仍合法。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/harness/install_launchd_agent.sh"
TEMPLATE="$ROOT/.harness/launchd/com.netmic.harness.coordinator.plist.example"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

home_dir="$tmpdir/home"
mkdir -p "$home_dir"

cat >"$tmpdir/hosts.env" <<EOF
NETMIC_HARNESS_COORDINATOR_ROOT=$ROOT
NETMIC_HARNESS_MAC_ROOT=$ROOT
NETMIC_HARNESS_LINUX_HOST=127.0.0.1
NETMIC_HARNESS_LINUX_PORT=22
NETMIC_HARNESS_LINUX_USER=tester
NETMIC_HARNESS_LINUX_ROOT=/home/tester/code/NetMic
NETMIC_HARNESS_SERVER_HOST=127.0.0.1
NETMIC_HARNESS_SERVER_PORT=43000
NETMIC_HARNESS_ARTIFACT_DIR=$tmpdir/runs
EOF

plutil -lint "$TEMPLATE" >/dev/null

HOME="$home_dir" "$TARGET" \
  --hosts-env "$tmpdir/hosts.env" \
  --until M3 \
  --interval-sec 1800 \
  --label com.netmic.harness.coordinator.test \
  --write-only >/dev/null

target_plist="$home_dir/Library/LaunchAgents/com.netmic.harness.coordinator.test.plist"
if [[ ! -f "$target_plist" ]]; then
  echo "缺少生成的 plist：$target_plist" >&2
  exit 1
fi

plutil -lint "$target_plist" >/dev/null

python3 - "$target_plist" "$tmpdir/hosts.env" <<'PY'
import plistlib
import sys
from pathlib import Path

path = Path(sys.argv[1])
hosts_env = sys.argv[2]
data = plistlib.loads(path.read_bytes())

assert data["Label"] == "com.netmic.harness.coordinator.test", data
assert data["StartInterval"] == 1800, data
assert data["ProgramArguments"][1:5] == ["--hosts-env", hosts_env, "--until", "M3"], data
assert data["StandardOutPath"].endswith(".harness/launchd/coordinator.stdout.log"), data
assert data["StandardErrorPath"].endswith(".harness/launchd/coordinator.stderr.log"), data
PY

echo "[ok] install_launchd_agent.sh 最小测试通过"
