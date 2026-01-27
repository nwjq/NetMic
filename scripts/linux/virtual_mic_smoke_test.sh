#!/usr/bin/env bash
# 使用 stub pactl/sox/paplay 验证 virtual_mic_smoke.sh 的最小行为。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/linux/virtual_mic_smoke.sh"

if [[ ! -x "$TARGET" ]]; then
  echo "目标脚本不存在或不可执行：$TARGET" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

stub_dir="$tmpdir/stubbin"
state_dir="$tmpdir/state"
mkdir -p "$stub_dir" "$state_dir"

cat >"$stub_dir/pactl" <<'PACTL'
#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${STATE_DIR:?}"

cmd="${1:-}"
shift || true

case "$cmd" in
  info)
    cat <<INFO
Server Name: PulseAudio (on PipeWire 0.3.79)
INFO
    ;;
  load-module)
    module="${1:-}"
    shift || true
    case "$module" in
      module-null-sink)
        echo "201"
        ;;
      module-remap-source)
        source_name=""
        for arg in "$@"; do
          case "$arg" in
            source_name=*)
              source_name="${arg#source_name=}"
              ;;
          esac
        done
        if [[ -n "$source_name" ]]; then
          printf '%s\n' "$source_name" >"$STATE_DIR/source_name"
        fi
        echo "202"
        ;;
      *)
        exit 1
        ;;
    esac
    ;;
  unload-module)
    exit 0
    ;;
  list)
    sub="${1:-}"
    shift || true
    if [[ "$sub" == "short" && "${1:-}" == "sources" ]]; then
      source_name="$(cat "$STATE_DIR/source_name" 2>/dev/null || true)"
      if [[ -n "$source_name" ]]; then
        printf '77\t%s\tmodule-remap-source\t...\n' "$source_name"
      fi
    fi
    ;;
  *)
    exit 1
    ;;
esac
PACTL

cat >"$stub_dir/sox" <<'SOX'
#!/usr/bin/env bash
set -euo pipefail

# 只需保证输出 wav 文件被创建。
out_file=""
prev=""
for arg in "$@"; do
  if [[ "$arg" == "synth" ]]; then
    out_file="$prev"
    break
  fi
  prev="$arg"
done

if [[ -z "$out_file" ]]; then
  echo "stub sox: 未找到输出文件参数" >&2
  exit 1
fi

touch "$out_file"
SOX

cat >"$stub_dir/paplay" <<'PAPLAY'
#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${STATE_DIR:?}"

device=""
wav=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --device)
      device="${2:-}"
      shift 2
      ;;
    *)
      wav="$1"
      shift
      ;;
  esac
done

if [[ -z "$device" || -z "$wav" ]]; then
  echo "stub paplay: 参数不完整" >&2
  exit 1
fi

printf '%s\n' "$device" >"$STATE_DIR/last_device"
printf '%s\n' "$wav" >"$STATE_DIR/last_wav"
PAPLAY

chmod +x "$stub_dir/pactl" "$stub_dir/sox" "$stub_dir/paplay"

echo "[test] pactl 缺失应失败"
if PATH="$stub_dir/empty" "$TARGET" >/dev/null 2>&1; then
  echo "期望失败但成功了（pactl 缺失场景）" >&2
  exit 1
fi

echo "[test] stub pactl + sox + paplay 应成功"
out_file="$tmpdir/out.txt"
if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_SMOKE_ID="test" "$TARGET" --duration 1 >"$out_file" 2>&1; then
  echo "stub smoke 测试失败，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi

if ! grep -q 'smoke 完成' "$out_file"; then
  echo "未看到 smoke 完成提示，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi

expected_device="netmic_smoke_sink_test"
actual_device="$(cat "$state_dir/last_device" 2>/dev/null || true)"
if [[ "$actual_device" != "$expected_device" ]]; then
  echo "paplay 目标设备不符合预期：$actual_device（期望 $expected_device）" >&2
  exit 1
fi

echo "[ok] virtual_mic_smoke.sh 最小测试通过"
