#!/usr/bin/env bash
# 使用 stub pactl 对 audio_selfcheck.sh 做最小可重复测试。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET="$ROOT/scripts/linux/audio_selfcheck.sh"

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
missing_dir="$tmpdir/missingbin"
mkdir -p "$missing_dir"
ln -s "$(command -v bash)" "$missing_dir/bash"

cat >"$stub_dir/pactl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

STATE_DIR="${STATE_DIR:?}"

cmd="${1:-}"
shift || true

case "$cmd" in
  info)
    cat <<INFO
Server Name: PulseAudio (on PipeWire 0.3.79)
Default Sink: netmic_stub_sink
Default Source: netmic_stub_source
INFO
    ;;
  load-module)
    module="${1:-}"
    shift || true
    case "$module" in
      module-null-sink)
        echo "101"
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
        echo "102"
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
        printf '42\t%s\tmodule-remap-source\t...\n' "$source_name"
      fi
    fi
    ;;
  *)
    exit 1
    ;;
esac
EOF

chmod +x "$stub_dir/pactl"

echo "[test] pactl 缺失时应判定 NOT_READY（--json）"
missing_json="$tmpdir/missing.json"
if PATH="$missing_dir" "$TARGET" --json >"$missing_json" 2>"$tmpdir/missing.stderr"; then
  echo "期望失败但成功了（pactl 缺失场景）" >&2
  cat "$missing_json" >&2 || true
  exit 1
fi
if ! grep -q '"pactl_available":false' "$missing_json"; then
  echo "pactl 缺失场景未输出 pactl_available=false：" >&2
  cat "$missing_json" >&2
  exit 1
fi
if ! grep -q '"conclusion":"NOT_READY"' "$missing_json"; then
  echo "pactl 缺失场景未输出 NOT_READY 结论：" >&2
  cat "$missing_json" >&2
  exit 1
fi

echo "[test] stub pactl + smoke 应判定 READY（--json）"
out_file="$tmpdir/out.json"
if ! PATH="$stub_dir:$PATH" STATE_DIR="$state_dir" NETMIC_SMOKE_ID="test" "$TARGET" --smoke --json >"$out_file" 2>"$tmpdir/out.stderr"; then
  echo "stub smoke 测试失败，输出如下：" >&2
  cat "$out_file" >&2 || true
  cat "$tmpdir/out.stderr" >&2 || true
  exit 1
fi

if ! grep -q '"pactl_available":true' "$out_file"; then
  echo "未看到 pactl_available=true，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi
if ! grep -q '"server_type":"pipewire-pulse"' "$out_file"; then
  echo "未看到 server_type=pipewire-pulse，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi
if ! grep -q '"default_sink":"netmic_stub_sink"' "$out_file"; then
  echo "未看到 default_sink，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi
if ! grep -q '"default_source":"netmic_stub_source"' "$out_file"; then
  echo "未看到 default_source，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi
if ! grep -q '"smoke_ok":true' "$out_file"; then
  echo "未看到 smoke_ok=true，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi
if ! grep -q '"conclusion":"READY"' "$out_file"; then
  echo "未看到 READY 结论，输出如下：" >&2
  cat "$out_file" >&2
  exit 1
fi

echo "[ok] audio_selfcheck.sh 最小测试通过"
