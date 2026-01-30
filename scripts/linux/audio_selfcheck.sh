#!/usr/bin/env bash
# Linux 音频注入自检：检查 PipeWire/Pulse 兼容层是否可用于创建虚拟麦克风。
set -euo pipefail

usage() {
  printf '%s\n' "用法："
  printf '%s\n' "  scripts/linux/audio_selfcheck.sh [--smoke] [--json]"
  printf '%s\n' ""
  printf '%s\n' "说明："
  printf '%s\n' "  --smoke  尝试创建临时虚拟麦克风，验证模块可用性"
  printf '%s\n' "  --json   以 machine-readable JSON 输出关键结论（日志转为 stderr）"
}

SMOKE=0
JSON=0
for arg in "${@:-}"; do
  case "$arg" in
    --smoke)
      SMOKE=1
      ;;
    --json)
      JSON=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      printf '未知参数：%s\n' "$arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

SMOKE_SINK_ID=""
SMOKE_SOURCE_ID=""
PACTL_AVAILABLE=0
SERVER_NAME=""
SERVER_KIND="unknown"
DEFAULT_SINK=""
DEFAULT_SOURCE=""
SMOKE_OK="null"
CONCLUSION=""
REASON=""

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
  if [[ "$JSON" -eq 1 ]]; then
    printf '%s\n' "$*" >&2
    return 0
  fi
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

pactl_info() {
  # 强制英文输出，避免本地化字段导致 awk 解析失败。
  LC_ALL=C pactl info 2>/dev/null
}

detect_server_name() {
  pactl_info | awk -F': ' '/^Server Name:/ {print $2; exit}'
}

detect_default_sink() {
  pactl_info | awk -F': ' '/^Default Sink:/ {print $2; exit}'
}

detect_default_source() {
  pactl_info | awk -F': ' '/^Default Source:/ {print $2; exit}'
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

json_escape() {
  local raw="$1"
  raw="${raw//\\/\\\\}"
  raw="${raw//\"/\\\"}"
  raw="${raw//$'\n'/ }"
  printf '%s' "$raw"
}

emit_json() {
  local pactl_available_json=false
  local smoke_requested_json=false
  if [[ "$PACTL_AVAILABLE" -eq 1 ]]; then
    pactl_available_json=true
  fi
  if [[ "$SMOKE" -eq 1 ]]; then
    smoke_requested_json=true
  fi

  printf '%s\n' "{\"ok\":true,\"pactl_available\":$pactl_available_json,\"server_name\":\"$(json_escape "$SERVER_NAME")\",\"server_type\":\"$(json_escape "$SERVER_KIND")\",\"default_sink\":\"$(json_escape "$DEFAULT_SINK")\",\"default_source\":\"$(json_escape "$DEFAULT_SOURCE")\",\"smoke_requested\":$smoke_requested_json,\"smoke_ok\":$SMOKE_OK,\"conclusion\":\"$(json_escape "$CONCLUSION")\",\"reason\":\"$(json_escape "$REASON")\"}"
}

basic_checks() {
  if command -v pactl >/dev/null 2>&1; then
    PACTL_AVAILABLE=1
  else
    REASON="缺少命令：pactl"
    log_fail "$REASON"
    return 1
  fi

  SERVER_NAME="$(detect_server_name || true)"
  DEFAULT_SINK="$(detect_default_sink || true)"
  DEFAULT_SOURCE="$(detect_default_source || true)"
  if [[ -z "$SERVER_NAME" ]]; then
    REASON="无法通过 pactl 读取服务端信息（Pulse/PipeWire 可能未运行）。"
    log_fail "无法通过 pactl 读取服务端信息（Pulse/PipeWire 可能未运行）。"
    return 1
  fi

  SERVER_KIND="$(server_kind_from_name "$SERVER_NAME")"
  log "检测到音频服务：$SERVER_NAME"
  case "$SERVER_KIND" in
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
    REASON="无法加载 module-null-sink（缺少模块或服务端拒绝）。"
    log_fail "无法加载 module-null-sink（缺少模块或服务端拒绝）。"
    return 1
  fi

  SMOKE_SOURCE_ID="$(pactl load-module module-remap-source \
    "master=${sink_name}.monitor" \
    "source_name=${source_name}" \
    "source_properties=device.description=NetMic_Smoke_Source" 2>/dev/null || true)"
  if [[ -z "$SMOKE_SOURCE_ID" ]]; then
    REASON="无法加载 module-remap-source（虚拟 source 创建失败）。"
    log_fail "无法加载 module-remap-source（虚拟 source 创建失败）。"
    return 1
  fi

  if pactl list short sources 2>/dev/null | awk '{print $2}' | grep -Fx "$source_name" >/dev/null 2>&1; then
    REASON=""
    log "smoke 检查通过：可创建虚拟麦克风（临时 source 已出现）。"
    return 0
  fi

  REASON="smoke 检查失败：未在 sources 列表中看到临时虚拟麦克风。"
  log_fail "smoke 检查失败：未在 sources 列表中看到临时虚拟麦克风。"
  return 1
}

main() {
  if ! basic_checks; then
    CONCLUSION="NOT_READY"
    if [[ -z "$REASON" ]]; then
      REASON="基础检查未通过"
    fi
    log "结论：NOT_READY（基础检查未通过）"
    if [[ "$JSON" -eq 1 ]]; then
      emit_json
    fi
    return 1
  fi

  if [[ "$SMOKE" -eq 1 ]]; then
    if smoke_check_virtual_source; then
      SMOKE_OK=true
      CONCLUSION="READY"
      log "结论：READY（smoke 检查通过）"
      if [[ "$JSON" -eq 1 ]]; then
        emit_json
      fi
      return 0
    fi
    SMOKE_OK=false
    CONCLUSION="NOT_READY"
    if [[ -z "$REASON" ]]; then
      REASON="smoke 检查失败"
    fi
    log "结论：NOT_READY（smoke 检查失败）"
    if [[ "$JSON" -eq 1 ]]; then
      emit_json
    fi
    return 1
  fi

  CONCLUSION="CHECK_OK"
  log "结论：CHECK_OK（基础检查通过，建议使用 --smoke 进一步验证）"
  if [[ "$JSON" -eq 1 ]]; then
    emit_json
  fi
}

main "$@"
