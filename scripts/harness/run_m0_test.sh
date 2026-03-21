#!/usr/bin/env bash
# run_m0.py 的最小自测：用 stub ssh 模拟远端 M0 场景。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/harness/run_m0.py"
RENDER="$ROOT/scripts/harness/render_ui_artifacts.mjs"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

stub_dir="$tmpdir/stubs"
artifact_root="$tmpdir/runs"
mkdir -p "$stub_dir" "$artifact_root"

cat >"$stub_dir/rsync" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
exit 0
EOF
chmod +x "$stub_dir/rsync"

cat >"$stub_dir/ssh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

joined="$*"
case "$joined" in
  *"&& pwd'"*)
    printf '/srv/netmic\n'
    exit 0
    ;;
  *"scripts/linux/audio_selfcheck.sh --json"*)
    printf '%s\n' '{"ok":true,"pactl_available":true,"server_name":"PulseAudio (on PipeWire 1.0.0)","server_type":"pipewire-pulse","default_sink":"alsa_output","default_source":"alsa_input","smoke_requested":false,"smoke_ok":null,"conclusion":"CHECK_OK","reason":""}'
    exit 0
    ;;
  *"scripts/linux/virtual_mic.sh create"*)
    printf '虚拟麦克风创建成功：netmic_source\n'
    exit 0
    ;;
  *"scripts/linux/virtual_mic.sh status --json"*)
    printf '%s\n' '{"ok":true,"action":"status","ready":true,"sink_name":"netmic_sink","source_name":"netmic_source","sink_module_id":"1","source_module_id":"2","state_file":"/tmp/netmic_virtual_mic.env"}'
    exit 0
    ;;
  *"scripts/linux/virtual_mic_smoke.sh --duration"*)
    printf '开始写入测试音（时长 1s）\n'
    printf 'smoke 完成：虚拟麦克风链路已走通（创建 + 写入尝试）\n'
    exit 0
    ;;
esac

printf 'unexpected ssh invocation: %s\n' "$*" >&2
exit 1
EOF
chmod +x "$stub_dir/ssh"

cat >"$tmpdir/hosts.env" <<EOF
NETMIC_HARNESS_COORDINATOR_ROOT=$ROOT
NETMIC_HARNESS_MAC_ROOT=$ROOT
NETMIC_HARNESS_LINUX_HOST=127.0.0.1
NETMIC_HARNESS_LINUX_PORT=22
NETMIC_HARNESS_LINUX_USER=tester
NETMIC_HARNESS_LINUX_ROOT=/srv/netmic
NETMIC_HARNESS_SERVER_HOST=127.0.0.1
NETMIC_HARNESS_SERVER_PORT=43000
NETMIC_HARNESS_ARTIFACT_DIR=$artifact_root
EOF

if [[ ! -f "$RENDER" ]]; then
  echo "缺少渲染脚本：$RENDER" >&2
  exit 1
fi

PATH="$stub_dir:$PATH" python3 "$TARGET" \
  --hosts-env "$tmpdir/hosts.env" \
  --run-id "m0-test" \
  --duration 1 >/dev/null

report="$artifact_root/m0-test/report.json"
visible="$artifact_root/m0-test/ui/visible-status.json"
if [[ ! -f "$report" ]]; then
  echo "缺少 report.json" >&2
  exit 1
fi
if [[ ! -f "$visible" ]]; then
  echo "缺少 visible-status.json" >&2
  exit 1
fi

python3 - "$report" "$visible" <<'PY'
import json
import sys

report_path, visible_path = sys.argv[1], sys.argv[2]
report = json.load(open(report_path, "r", encoding="utf-8"))
visible = json.load(open(visible_path, "r", encoding="utf-8"))

assert report["status"] == "pass", report
assert any(step["id"] == "ui-verify" and step["status"] == "pass" for step in report["steps"]), report
assert "虚拟麦克风：已就绪" in "\n".join(visible["connection_lines"]), visible
PY

echo "[ok] run_m0.py 最小测试通过"
