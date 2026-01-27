#!/usr/bin/env bash
# Linux 音频注入自检：检查 PipeWire/Pulse 兼容层是否可用于创建虚拟麦克风。
set -euo pipefail

SMOKE=0
if [[ "${1:-}" == "--smoke" ]]; then
  SMOKE=1
fi

SMOKE_SINK_ID=""
SMOKE_SOURCE_ID=""

smoke_cleanup() {
  # 使用默认展开，避免 set -u 在未绑定变量时退出。
  local source_id="${SMOKE_SOURCE_ID:-}"
  local sink_id="${SMOKE_SINK_ID:-}"
  if [[ -n "$source_id" ]]; then
    pactl unload-module "$source_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$sink_id" ]]; then
    pactl unload-module "$sink_id" >/dev/null 2>&1 || true
  fi
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

detect_server_name() {
  pactl info 2>/dev/null | awk -F': ' '/^Server Name:/ {print $2; exit}'
}

server_kind_from_name() {
  local name="$1"
  if [[ "$name" == *"PipeWire"* ]] || [[ "$name" == *"on PipeWire"* ]]; then
    printf 'pipewire-pulse'
    return 0
  fi
  if [[ "$name" == *"PulseAudio"* ]] || [[ "$name" == *"pulseaudio"* ]]; then
    printf 'pulseaudio'
    return 0
  fi
  printf 'unknown'
}

basic_checks() {
  require_cmd pactl || return 1
  local server_name
  server_name="$(detect_server_name || true)"
  if [[ -z "$server_name" ]]; then
    log_fail "无法通过 pactl 读取服务端信息（Pulse/PipeWire 可能未运行）。"
    return 1
  fi

  local server_kind
  server_kind="$(server_kind_from_name "$server_name")"
  log "检测到音频服务：$server_name"
  case "$server_kind" in
    pipewire-pulse)
      log "服务类型判断：PipeWire（Pulse 兼容层）"
      ;;
    pulseaudio)
      log "服务类型判断：PulseAudio"
      ;;
    *)
      log_warn "服务类型未识别，但仍可尝试进行 smoke 检查。"
      ;;
  esac
}

smoke_check_virtual_source() {
  local suffix
  suffix="${NETMIC_SMOKE_ID:-$$}"
  local sink_name="netmic_smoke_sink_${suffix}"
  local source_name="netmic_smoke_source_${suffix}"

  # trap 在函数返回后仍会执行，因此这里先重置脚本级状态。
  SMOKE_SINK_ID=""
  SMOKE_SOURCE_ID=""
  trap smoke_cleanup EXIT

  log "开始 smoke 检查：尝试创建临时虚拟麦克风。"

  # 优先使用 module-null-sink + module-remap-source 的组合，兼容性更好。
  SMOKE_SINK_ID="$(pactl load-module module-null-sink \
    "sink_name=${sink_name}" \
    "sink_properties=device.description=NetMic_Smoke_Sink" 2>/dev/null || true)"
  if [[ -z "$SMOKE_SINK_ID" ]]; then
    log_fail "无法加载 module-null-sink（缺少模块或服务端拒绝）。"
    return 1
  fi

  SMOKE_SOURCE_ID="$(pactl load-module module-remap-source \
    "master=${sink_name}.monitor" \
    "source_name=${source_name}" \
    "source_properties=device.description=NetMic_Smoke_Source" 2>/dev/null || true)"
  if [[ -z "$SMOKE_SOURCE_ID" ]]; then
    log_fail "无法加载 module-remap-source（虚拟 source 创建失败）。"
    return 1
  fi

  if pactl list short sources 2>/dev/null | awk '{print $2}' | grep -Fx "$source_name" >/dev/null 2>&1; then
    log "smoke 检查通过：可创建虚拟麦克风（临时 source 已出现）。"
    return 0
  fi

  log_fail "smoke 检查失败：未在 sources 列表中看到临时虚拟麦克风。"
  return 1
}

main() {
  if ! basic_checks; then
    log "结论：NOT_READY（基础检查未通过）"
    return 1
  fi

  if [[ "$SMOKE" -eq 1 ]]; then
    if smoke_check_virtual_source; then
      log "结论：READY（smoke 检查通过）"
      return 0
    fi
    log "结论：NOT_READY（smoke 检查失败）"
    return 1
  fi

  log "结论：CHECK_OK（基础检查通过，建议使用 --smoke 进一步验证）"
}

main "$@"
