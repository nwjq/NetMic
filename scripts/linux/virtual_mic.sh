#!/usr/bin/env bash
# Linux 虚拟麦克风创建/移除脚本（PipeWire / Pulse 兼容层）。
# 目标：为 M0 提供可复用、可自检、幂等的虚拟麦克风入口。
set -euo pipefail

usage() {
  cat <<'EOF'
用法：
  scripts/linux/virtual_mic.sh [create|remove|status] [--json]

环境变量（可选）：
  NETMIC_VIRTUAL_MIC_STATE        状态文件路径（默认 /tmp/netmic_virtual_mic.env）
  NETMIC_VIRTUAL_MIC_PREFIX       设备名前缀（默认 netmic）
  NETMIC_VIRTUAL_MIC_SINK_NAME    虚拟 sink 名称（默认 <prefix>_sink）
  NETMIC_VIRTUAL_MIC_SOURCE_NAME  虚拟 source 名称（默认 <prefix>_source）
EOF
}

ACTION=""
JSON=0
for arg in "${@:-}"; do
  case "$arg" in
    create|remove|status)
      ACTION="$arg"
      ;;
    --json)
      JSON=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数：$arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done
ACTION="${ACTION:-status}"

STATE_FILE="${NETMIC_VIRTUAL_MIC_STATE:-/tmp/netmic_virtual_mic.env}"
NAME_PREFIX="${NETMIC_VIRTUAL_MIC_PREFIX:-netmic}"
SINK_NAME="${NETMIC_VIRTUAL_MIC_SINK_NAME:-${NAME_PREFIX}_sink}"
SOURCE_NAME="${NETMIC_VIRTUAL_MIC_SOURCE_NAME:-${NAME_PREFIX}_source}"
SINK_DESCRIPTION="${NETMIC_VIRTUAL_MIC_SINK_DESC:-NetMic_Virtual_Sink}"
SOURCE_DESCRIPTION="${NETMIC_VIRTUAL_MIC_SOURCE_DESC:-NetMic_Virtual_Mic}"
SAMPLE_RATE_HZ=48000
CHANNELS=1
SAMPLE_FORMAT=s16le
CHANNEL_MAP=mono

log() {
  if [[ "$JSON" -eq 0 ]]; then
    printf '%s\n' "$*"
  fi
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

source_exists() {
  pactl list short sources 2>/dev/null | awk '{print $2}' | grep -Fx "$SOURCE_NAME" >/dev/null 2>&1
}

source_matches_internal_format() {
  pactl list short sources 2>/dev/null \
    | awk -v name="$SOURCE_NAME" -v format="$SAMPLE_FORMAT" -v channels="${CHANNELS}ch" -v rate="${SAMPLE_RATE_HZ}Hz" '
        $2 == name && $4 == format && $5 == channels && $6 == rate { found=1 }
        END { exit(found ? 0 : 1) }
      '
}

find_module_id() {
  local module_name="$1"
  local match_arg="$2"
  pactl list short modules 2>/dev/null \
    | awk -v module="$module_name" -v needle="$match_arg" '$2 == module && index($0, needle) {print $1; exit}'
}

read_state() {
  if [[ -f "$STATE_FILE" ]]; then
    # shellcheck disable=SC1090
    . "$STATE_FILE"
  fi
  SINK_MODULE_ID="${SINK_MODULE_ID:-}"
  SOURCE_MODULE_ID="${SOURCE_MODULE_ID:-}"
}

write_state() {
  cat >"$STATE_FILE" <<EOF
SINK_MODULE_ID="${SINK_MODULE_ID:-}"
SOURCE_MODULE_ID="${SOURCE_MODULE_ID:-}"
SINK_NAME="$SINK_NAME"
SOURCE_NAME="$SOURCE_NAME"
EOF
}

cleanup_modules() {
  local source_id="${SOURCE_MODULE_ID:-}"
  local sink_id="${SINK_MODULE_ID:-}"
  if [[ -n "$source_id" ]]; then
    pactl unload-module "$source_id" >/dev/null 2>&1 || true
  fi
  if [[ -n "$sink_id" ]]; then
    pactl unload-module "$sink_id" >/dev/null 2>&1 || true
  fi
}

emit_status_json() {
  local ready="$1"
  local sink_id="$2"
  local source_id="$3"
  local state_file="$4"
  cat <<EOF
{"ok":true,"action":"status","ready":$ready,"sink_name":"$SINK_NAME","source_name":"$SOURCE_NAME","sink_module_id":"$sink_id","source_module_id":"$source_id","state_file":"$state_file"}
EOF
}

status_action() {
  read_state
  local sink_id="${SINK_MODULE_ID:-}"
  local source_id="${SOURCE_MODULE_ID:-}"

  if [[ -z "$sink_id" ]]; then
    sink_id="$(find_module_id module-null-sink "sink_name=${SINK_NAME}")"
  fi
  if [[ -z "$source_id" ]]; then
    source_id="$(find_module_id module-remap-source "source_name=${SOURCE_NAME}")"
  fi

  if source_exists; then
    log "虚拟麦克风已就绪：$SOURCE_NAME"
    log "关联 sink：$SINK_NAME"
    log "模块 id：sink=${sink_id:-unknown}, source=${source_id:-unknown}"
    log "状态文件：$STATE_FILE"
    if [[ "$JSON" -eq 1 ]]; then
      emit_status_json true "${sink_id:-}" "${source_id:-}" "$STATE_FILE"
    fi
    return 0
  fi

  log_warn "未检测到虚拟麦克风：$SOURCE_NAME"
  log_warn "可尝试执行：scripts/linux/virtual_mic.sh create"
  if [[ "$JSON" -eq 1 ]]; then
    emit_status_json false "${sink_id:-}" "${source_id:-}" "$STATE_FILE"
  fi
  return 1
}

create_action() {
  read_state

  if source_exists; then
    if ! source_matches_internal_format; then
      log_warn "检测到旧格式虚拟麦克风，先重建为 ${SAMPLE_FORMAT}/${CHANNELS}ch/${SAMPLE_RATE_HZ}Hz"
      cleanup_modules
      rm -f "$STATE_FILE"
    else
    # 注意：在 UTF-8 locale 下，中文括号可能被 bash 误判为变量名的一部分。
    # 使用 ${VAR} 形式避免 set -u 误报 unbound variable。
      log "检测到已存在虚拟麦克风：${SOURCE_NAME}（执行幂等 create）"
      if [[ -z "${SINK_MODULE_ID:-}" ]]; then
        SINK_MODULE_ID="$(find_module_id module-null-sink "sink_name=${SINK_NAME}")"
      fi
      if [[ -z "${SOURCE_MODULE_ID:-}" ]]; then
        SOURCE_MODULE_ID="$(find_module_id module-remap-source "source_name=${SOURCE_NAME}")"
      fi
      write_state
      return 0
    fi
  fi

  SINK_MODULE_ID="$(pactl load-module module-null-sink \
    "sink_name=${SINK_NAME}" \
    "sink_properties=device.description=${SINK_DESCRIPTION}" \
    "format=${SAMPLE_FORMAT}" \
    "rate=${SAMPLE_RATE_HZ}" \
    "channels=${CHANNELS}" \
    "channel_map=${CHANNEL_MAP}" 2>/dev/null || true)"
  if [[ -z "$SINK_MODULE_ID" ]]; then
    log_fail "无法加载 module-null-sink（创建虚拟 sink 失败）。"
    return 1
  fi

  SOURCE_MODULE_ID="$(pactl load-module module-remap-source \
    "master=${SINK_NAME}.monitor" \
    "source_name=${SOURCE_NAME}" \
    "source_properties=device.description=${SOURCE_DESCRIPTION}" \
    "format=${SAMPLE_FORMAT}" \
    "rate=${SAMPLE_RATE_HZ}" \
    "channels=${CHANNELS}" \
    "channel_map=${CHANNEL_MAP}" \
    "master_channel_map=${CHANNEL_MAP}" 2>/dev/null || true)"
  if [[ -z "$SOURCE_MODULE_ID" ]]; then
    log_fail "无法加载 module-remap-source（创建虚拟麦克风失败）。"
    cleanup_modules
    return 1
  fi

  if ! source_exists; then
    log_fail "创建后仍未在 sources 列表看到：$SOURCE_NAME"
    cleanup_modules
    return 1
  fi

  write_state
  log "虚拟麦克风创建成功：$SOURCE_NAME"
  log "模块 id：sink=$SINK_MODULE_ID, source=$SOURCE_MODULE_ID"
  log "可用状态检查：scripts/linux/virtual_mic.sh status"
  return 0
}

remove_action() {
  read_state

  if [[ -z "${SINK_MODULE_ID:-}" ]]; then
    SINK_MODULE_ID="$(find_module_id module-null-sink "sink_name=${SINK_NAME}")"
  fi
  if [[ -z "${SOURCE_MODULE_ID:-}" ]]; then
    SOURCE_MODULE_ID="$(find_module_id module-remap-source "source_name=${SOURCE_NAME}")"
  fi

  if [[ -z "${SINK_MODULE_ID:-}" && -z "${SOURCE_MODULE_ID:-}" && ! -f "$STATE_FILE" ]]; then
    log "未发现可移除的模块（已是干净状态）。"
    return 0
  fi

  cleanup_modules
  rm -f "$STATE_FILE"

  if source_exists; then
    log_fail "已尝试移除，但仍检测到虚拟麦克风：$SOURCE_NAME"
    return 1
  fi

  log "虚拟麦克风已移除：$SOURCE_NAME"
  return 0
}

main() {
  require_cmd pactl || return 1

  case "$ACTION" in
    status)
      status_action
      ;;
    create)
      create_action
      ;;
    remove)
      remove_action
      ;;
    *)
      usage >&2
      return 2
      ;;
  esac
}

main
