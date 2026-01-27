#!/usr/bin/env bash
# Linux 虚拟麦克风 smoke：创建虚拟 source 并尝试写入测试音（sine wave）。
set -euo pipefail

DURATION_SEC="${NETMIC_SMOKE_DURATION_SEC:-30}"
SUFFIX="${NETMIC_SMOKE_ID:-$$}"
PREFIX="${NETMIC_SMOKE_PREFIX:-netmic_smoke}"
KEEP_MODULES=0
CLEANUP_ONLY=0

SINK_MODULE_ID=""
SOURCE_MODULE_ID=""

usage() {
  cat <<USAGE
用法：$(basename "$0") [--duration <seconds>] [--keep-modules] [--cleanup-only]

选项：
  --duration <seconds>  测试音时长（秒），默认 30（可用 NETMIC_SMOKE_DURATION_SEC 覆盖）
  --keep-modules        结束时不自动卸载模块（便于手动观察）
  --cleanup-only        仅创建并验证虚拟麦克风是否出现，然后立即退出
  -h, --help            显示帮助
USAGE
}

log() {
  printf '%s\n' "$*"
}

log_warn() {
  printf 'WARN: %s\n' "$*" >&2
}

log_fail() {
  printf 'FAIL: %s\n' "$*" >&2
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    log_fail "缺少命令：$cmd"
    return 1
  fi
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --duration)
        if [[ $# -lt 2 ]]; then
          log_fail "--duration 需要一个秒数参数"
          return 1
        fi
        DURATION_SEC="$2"
        shift 2
        ;;
      --keep-modules)
        KEEP_MODULES=1
        shift
        ;;
      --cleanup-only)
        CLEANUP_ONLY=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        log_fail "未知参数：$1"
        usage >&2
        return 1
        ;;
    esac
  done
}

sink_name() {
  printf '%s_sink_%s' "$PREFIX" "$SUFFIX"
}

source_name() {
  printf '%s_source_%s' "$PREFIX" "$SUFFIX"
}

cleanup() {
  if [[ "$KEEP_MODULES" -eq 1 ]]; then
    log_warn "--keep-modules 已启用，跳过自动清理"
    return 0
  fi

  local source_id="${SOURCE_MODULE_ID:-}"
  local sink_id="${SINK_MODULE_ID:-}"

  if [[ -n "$source_id" ]]; then
    pactl unload-module "$source_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$sink_id" ]]; then
    pactl unload-module "$sink_id" >/dev/null 2>&1 || true
  fi
}

basic_checks() {
  require_cmd pactl || return 1

  if ! pactl info >/dev/null 2>&1; then
    log_fail "pactl info 失败：Pulse/PipeWire 可能未运行"
    return 1
  fi
}

create_virtual_mic() {
  local sink
  local source
  sink="$(sink_name)"
  source="$(source_name)"

  log "创建虚拟 sink：$sink"
  SINK_MODULE_ID="$(pactl load-module module-null-sink \
    "sink_name=${sink}" \
    "sink_properties=device.description=NetMic_Smoke_Sink" 2>/dev/null || true)"

  if [[ -z "$SINK_MODULE_ID" ]]; then
    log_fail "无法加载 module-null-sink"
    return 1
  fi

  log "创建虚拟 source：$source"
  SOURCE_MODULE_ID="$(pactl load-module module-remap-source \
    "master=${sink}.monitor" \
    "source_name=${source}" \
    "source_properties=device.description=NetMic_Smoke_Source" 2>/dev/null || true)"

  if [[ -z "$SOURCE_MODULE_ID" ]]; then
    log_fail "无法加载 module-remap-source"
    return 1
  fi

  if pactl list short sources 2>/dev/null | awk '{print $2}' | grep -Fx "$source" >/dev/null 2>&1; then
    log "虚拟麦克风已出现：$source"
    return 0
  fi

  log_fail "未在 sources 列表中看到虚拟麦克风：$source"
  return 1
}

generate_sine_wav() {
  local out_wav="$1"
  require_cmd sox || return 1

  # 生成固定时长的 48kHz/mono/16-bit 测试音，贴近内部标准格式。
  sox -n \
    -r 48000 \
    -c 1 \
    -b 16 \
    -e signed-integer \
    "$out_wav" \
    synth "$DURATION_SEC" sine 440 >/dev/null 2>&1
}

play_into_sink() {
  local wav_file="$1"
  local sink
  sink="$(sink_name)"

  if command -v paplay >/dev/null 2>&1; then
    log "使用 paplay 向 sink 写入测试音：$sink"
    paplay --device "$sink" "$wav_file" >/dev/null 2>&1
    return 0
  fi

  log_warn "缺少 paplay，跳过音频写入（仅验证虚拟麦克风创建）"
  return 0
}

run_smoke_audio() {
  local wav_file
  # 兼容 macOS（BSD mktemp 不支持 --suffix）。
  wav_file="$(mktemp "${TMPDIR:-/tmp}/netmic_smoke_XXXXXX")"

  if ! generate_sine_wav "$wav_file"; then
    rm -f "$wav_file"
    log_warn "缺少 sox 或生成测试音失败，跳过音频写入"
    return 0
  fi

  if ! play_into_sink "$wav_file"; then
    rm -f "$wav_file"
    return 1
  fi

  rm -f "$wav_file"
}

main() {
  parse_args "$@"

  basic_checks || return 1
  trap cleanup EXIT

  create_virtual_mic || return 1

  if [[ "$CLEANUP_ONLY" -eq 1 ]]; then
    log "cleanup-only 完成：虚拟麦克风可创建"
    return 0
  fi

  log "开始写入测试音（时长 ${DURATION_SEC}s）"
  run_smoke_audio
  log "smoke 完成：虚拟麦克风链路已走通（创建 + 写入尝试）"
}

main "$@"
