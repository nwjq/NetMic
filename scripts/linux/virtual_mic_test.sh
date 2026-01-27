#!/usr/bin/env bash
# 使用 stub pactl 验证 virtual_mic.sh 的 create/remove/status 幂等行为。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/linux/virtual_mic.sh"

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
state_file="$state_dir/virtual_mic.env"
mkdir -p "$stub_dir" "$state_dir"

cat >"$stub_dir/pactl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${STATE_DIR:?}"
SINK_ID="201"
SOURCE_ID="202"

sink_name_file="$STATE_DIR/sink_name"
source_name_file="$STATE_DIR/source_name"

cmd="${1:-}"
shift || true

read_arg_value() {
  local prefix="$1"
  shift || true
  for arg in "$@"; do
    case "$arg" in
      "$prefix"=*)
        printf '%s\n' "${arg#"$prefix"=}"
        return 0
        ;;
    esac
  done
  return 1
}

case "$cmd" in
  load-module)
    module="${1:-}"
    shift || true
    case "$module" in
      module-null-sink)
        sink_name="$(read_arg_value sink_name "$@" || true)"
        if [[ -n "${sink_name:-}" ]]; then
          printf '%s\n' "$sink_name" >"$sink_name_file"
        fi
        echo "$SINK_ID"
        ;;
      module-remap-source)
        source_name="$(read_arg_value source_name "$@" || true)"
        if [[ -n "${source_name:-}" ]]; then
          printf '%s\n' "$source_name" >"$source_name_file"
        fi
        echo "$SOURCE_ID"
        ;;
      *)
        exit 1
        ;;
    esac
    ;;
  list)
    sub="${1:-}"
    shift || true
    case "$sub" in
      short)
        kind="${1:-}"
        shift || true
        case "$kind" in
          sources)
            if [[ -f "$source_name_file" ]]; then
              source_name="$(cat "$source_name_file")"
              printf '88\t%s\tmodule-remap-source\t...\n' "$source_name"
            fi
            ;;
          modules)
            if [[ -f "$sink_name_file" ]]; then
              sink_name="$(cat "$sink_name_file")"
              printf '%s\tmodule-null-sink\tsink_name=%s sink_properties=device.description=NetMic_Virtual_Sink\n' "$SINK_ID" "$sink_name"
            fi
            if [[ -f "$source_name_file" ]]; then
              source_name="$(cat "$source_name_file")"
              printf '%s\tmodule-remap-source\tsource_name=%s master=netmic_sink.monitor source_properties=device.description=NetMic_Virtual_Mic\n' "$SOURCE_ID" "$source_name"
            fi
            ;;
        esac
        ;;
    esac
    ;;
  unload-module)
    module_id="${1:-}"
    case "$module_id" in
      "$SINK_ID")
        rm -f "$sink_name_file"
        ;;
      "$SOURCE_ID")
        rm -f "$source_name_file"
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
EOF

chmod +x "$stub_dir/pactl"

echo "[test] pactl 缺失时 create 应失败"
if PATH="$stub_dir/empty" "$TARGET" create >/dev/null 2>&1; then
  echo "期望失败但成功了（pactl 缺失场景）" >&2
  exit 1
fi

echo "[test] create → status → create(幂等) → remove → status(失败)"
out_create="$tmpdir/create.out"
if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_VIRTUAL_MIC_STATE="$state_file" "$TARGET" create >"$out_create" 2>&1; then
  echo "create 失败，输出如下：" >&2
  cat "$out_create" >&2
  exit 1
fi

if ! grep -q '创建成功' "$out_create"; then
  echo "未看到创建成功提示，输出如下：" >&2
  cat "$out_create" >&2
  exit 1
fi

if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_VIRTUAL_MIC_STATE="$state_file" "$TARGET" status >/dev/null 2>&1; then
  echo "status 期望成功但失败了" >&2
  exit 1
fi

if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_VIRTUAL_MIC_STATE="$state_file" "$TARGET" create >/dev/null 2>&1; then
  echo "幂等 create 失败" >&2
  exit 1
fi

if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_VIRTUAL_MIC_STATE="$state_file" "$TARGET" remove >/dev/null 2>&1; then
  echo "remove 失败" >&2
  exit 1
fi

if PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_VIRTUAL_MIC_STATE="$state_file" "$TARGET" status >/dev/null 2>&1; then
  echo "status 期望失败但成功了（remove 后场景）" >&2
  exit 1
fi

echo "[ok] virtual_mic.sh 最小测试通过"

