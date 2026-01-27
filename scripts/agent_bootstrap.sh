#!/usr/bin/env bash
# NetMic 自动驾驶入口与上下文加载器。
#
# 设计目标：
# 1) 用户只需运行本脚本即可启动自动开发流程（前台监督，不隐藏）。
# 2) 在 Agent 循环内（ROLE/BOOTSTRAP_CONTEXT_ONLY）退化为“上下文摘要器”。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STATE_DIR="$ROOT/.autopilot"
LOG_DIR="$STATE_DIR/logs"
RUNTIME_HUB_URL_FILE="$STATE_DIR/runtime_hub_url.txt"
RUNNER_ENV_FILE="$STATE_DIR/runner.env"
RUNNER_ENV_EXAMPLE="$STATE_DIR/runner.env.example"
HUB_URL_SHARED_FILE="$ROOT/docs/HUB_URL.txt"
BOOTSTRAP_LOG="$LOG_DIR/bootstrap.log"
WORKSPACE_MARKER="$STATE_DIR/workspace_bootstrap.done"
SUPERVISOR_STATE_FILE="$STATE_DIR/supervisor_state.env"
VERIFY_GATES_FILE="${BOOTSTRAP_VERIFY_GATES_FILE:-$ROOT/docs/MVP_GATES.yaml}"
VERIFY_JSON="$STATE_DIR/verify_status.json"
VERIFY_TEXT="$STATE_DIR/verify_status.txt"
RESEARCH_MODE_FILE="$STATE_DIR/research_mode.json"
RESEARCH_MODE_TEXT="$STATE_DIR/research_mode.txt"
RESTART_REQUEST_FILE="$STATE_DIR/restart.requested"

REFRESH_SECONDS="${BOOTSTRAP_REFRESH_SECONDS:-15}"
CONTEXT_ONLY="${BOOTSTRAP_CONTEXT_ONLY:-0}"
ONCE=0
STATUS_ONLY=0
STOP_ONLY=0
# verify/gate 轮询与研究/重启护栏参数。
VERIFY_INTERVAL_SECONDS="${BOOTSTRAP_VERIFY_INTERVAL_SECONDS:-300}"
RESEARCH_TRIGGER_REPEATS="${BOOTSTRAP_RESEARCH_TRIGGER_REPEATS:-3}"
RESEARCH_COOLDOWN_SECONDS="${BOOTSTRAP_RESEARCH_COOLDOWN_SECONDS:-1800}"
RESTART_COOLDOWN_SECONDS="${BOOTSTRAP_RESTART_COOLDOWN_SECONDS:-120}"
RESTART_LIMIT_PER_HOUR="${BOOTSTRAP_RESTART_LIMIT_PER_HOUR:-4}"
# 是否允许脚本自动安装 Rust 工具链（默认开启，可显式设为 0 关闭）。
AUTO_INSTALL_RUST="${NETMIC_AUTO_INSTALL_RUST:-1}"
RUSTUP_PROFILE="${NETMIC_RUSTUP_PROFILE:-minimal}"

usage() {
  cat <<'USAGE'
用法：scripts/agent_bootstrap.sh [--context] [--once] [--status] [--stop]

说明：
- 无参数：以前台监督器模式运行（推荐入口）。
- --context：只输出上下文摘要（供 agent 循环调用）。
- --once：执行一次自检/启动/状态汇总后退出（便于 CI 或调试）。
- --status：只看当前状态，不做启动动作。
- --stop：停止 autopilot 与本机 Hub。
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --context)
      CONTEXT_ONLY=1
      shift
      ;;
    --once)
      ONCE=1
      shift
      ;;
    --status)
      STATUS_ONLY=1
      shift
      ;;
    --stop)
      STOP_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数：$1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

mkdir -p "$STATE_DIR" "$LOG_DIR"

ts_now() {
  date -Iseconds
}

offline_tasks_summary() {
  if ! command -v sqlite3 >/dev/null 2>&1; then
    return 0
  fi
  if [[ ! -f "$ROOT/agent_hub.db" ]]; then
    return 0
  fi

  echo "-- HUB tasks (offline agent_hub.db) --"
  # 受限环境可能无法访问 Hub；此处退化为读取本地 agent_hub.db。
  sqlite3 "$ROOT/agent_hub.db" \
    "select task_id,status,coalesce(claimed_by,''),datetime(updated_at,'unixepoch') from tasks order by updated_at desc limit 10;" \
    || true
  echo
}

# -------- 监督器状态与 Gate 验证 --------
SUPERVISOR_LAST_VERIFY_TS=0
SUPERVISOR_LAST_FAILURE_SIGNATURE=""
SUPERVISOR_FAILURE_REPEAT_COUNT=0
SUPERVISOR_LAST_RESEARCH_TS=0
SUPERVISOR_LAST_RESTART_TS=0
SUPERVISOR_RESTART_WINDOW_START_TS=0
SUPERVISOR_RESTART_COUNT=0
SUPERVISOR_LAST_PROGRESS_SIGNATURE=""
SUPERVISOR_LAST_OVERALL_STATUS=""
SUPERVISOR_LAST_ACTIVE_GATE=""

load_supervisor_state() {
  # 先给默认值，避免 set -u 触发未绑定变量。
  SUPERVISOR_LAST_VERIFY_TS=0
  SUPERVISOR_LAST_FAILURE_SIGNATURE=""
  SUPERVISOR_FAILURE_REPEAT_COUNT=0
  SUPERVISOR_LAST_RESEARCH_TS=0
  SUPERVISOR_LAST_RESTART_TS=0
  SUPERVISOR_RESTART_WINDOW_START_TS=0
  SUPERVISOR_RESTART_COUNT=0
  SUPERVISOR_LAST_PROGRESS_SIGNATURE=""
  SUPERVISOR_LAST_OVERALL_STATUS=""
  SUPERVISOR_LAST_ACTIVE_GATE=""
  if [[ -f "$SUPERVISOR_STATE_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$SUPERVISOR_STATE_FILE"
  fi
}

save_supervisor_state() {
  cat >"$SUPERVISOR_STATE_FILE" <<EOF
SUPERVISOR_LAST_VERIFY_TS=${SUPERVISOR_LAST_VERIFY_TS:-0}
SUPERVISOR_LAST_FAILURE_SIGNATURE=${SUPERVISOR_LAST_FAILURE_SIGNATURE:-}
SUPERVISOR_FAILURE_REPEAT_COUNT=${SUPERVISOR_FAILURE_REPEAT_COUNT:-0}
SUPERVISOR_LAST_RESEARCH_TS=${SUPERVISOR_LAST_RESEARCH_TS:-0}
SUPERVISOR_LAST_RESTART_TS=${SUPERVISOR_LAST_RESTART_TS:-0}
SUPERVISOR_RESTART_WINDOW_START_TS=${SUPERVISOR_RESTART_WINDOW_START_TS:-0}
SUPERVISOR_RESTART_COUNT=${SUPERVISOR_RESTART_COUNT:-0}
SUPERVISOR_LAST_PROGRESS_SIGNATURE=${SUPERVISOR_LAST_PROGRESS_SIGNATURE:-}
SUPERVISOR_LAST_OVERALL_STATUS=${SUPERVISOR_LAST_OVERALL_STATUS:-}
SUPERVISOR_LAST_ACTIVE_GATE=${SUPERVISOR_LAST_ACTIVE_GATE:-}
EOF
}

clear_research_mode() {
  rm -f "$RESEARCH_MODE_FILE" "$RESEARCH_MODE_TEXT"
}

parse_verify_json() {
  local json_file="$1"
  if [[ ! -f "$json_file" ]]; then
    return 1
  fi
  python3 - "$json_file" <<'PY'
import json
import shlex
import sys

path = sys.argv[1]
try:
    data = json.load(open(path, "r", encoding="utf-8"))
except Exception:
    sys.exit(1)

overall = data.get("overall", {}) or {}
failure = overall.get("failure") or {}

def emit(key: str, value):
    if value is None:
        return
    print(f"{key}={shlex.quote(str(value))}")

emit("VERIFY_GENERATED_AT", data.get("generated_at"))
emit("VERIFY_OVERALL_STATUS", overall.get("overall_status"))
emit("VERIFY_ACTIVE_GATE", overall.get("active_gate_id"))
emit("VERIFY_PROGRESS_SIGNATURE", overall.get("progress_signature"))
emit("VERIFY_FAILURE_SIGNATURE", overall.get("failure_signature"))
emit("VERIFY_FAILURE_GATE", failure.get("gate_id"))
emit("VERIFY_FAILURE_CHECK", failure.get("check_id"))
emit("VERIFY_FAILURE_STATUS", failure.get("status"))
emit("VERIFY_FAILURE_EXIT", failure.get("exit_code"))
emit("VERIFY_FAILURE_HINT", failure.get("hint"))
emit("VERIFY_FAILURE_RUN", failure.get("run"))
PY
}

trigger_research_mode() {
  local reason="$1"
  if [[ ! -f "$VERIFY_JSON" ]]; then
    return 1
  fi
  python3 - "$VERIFY_JSON" "$RESEARCH_MODE_FILE" "$RESEARCH_MODE_TEXT" "$reason" <<'PY'
import json
import sys
from datetime import datetime, timezone

verify_path, research_json, research_txt, reason = sys.argv[1:5]

def now():
    return datetime.now(timezone.utc).astimezone().isoformat(timespec="seconds")

try:
    data = json.load(open(verify_path, "r", encoding="utf-8"))
except Exception as exc:
    payload = {"ok": False, "error": str(exc), "reason": reason, "generated_at": now()}
    open(research_json, "w", encoding="utf-8").write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    open(research_txt, "w", encoding="utf-8").write("research mode: failed to read verify_status.json\n")
    sys.exit(0)

overall = data.get("overall", {}) or {}
failure = overall.get("failure") or {}

payload = {
    "ok": True,
    "triggered_at": now(),
    "reason": reason,
    "overall_status": overall.get("overall_status"),
    "active_gate_id": overall.get("active_gate_id"),
    "failure_signature": overall.get("failure_signature"),
    "failure": failure,
    "instructions": [
        "优先查官方文档/主仓库/primary sources，避免二手答案。",
        "先解释失败签名与根因假设，再给出最小可验证修复。",
        "修复后必须重新运行 scripts/verify_mvp.py 验证 Gate 状态。",
        "若修改 scripts/agent_bootstrap.sh 或 scripts/autopilot.sh，请写入 .autopilot/restart.requested 请求热重启。",
    ],
}

open(research_json, "w", encoding="utf-8").write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")

lines = [
    "== NetMic 研究模式（Research Mode）==",
    f"time:    {payload['triggered_at']}",
    f"reason:  {reason}",
    f"status:  {payload.get('overall_status')}",
    f"active:  {payload.get('active_gate_id')}",
    f"sign:    {payload.get('failure_signature')}",
    "",
]

if failure:
    lines.extend(
        [
            "-- failure --",
            f"gate/check: {failure.get('gate_id')}/{failure.get('check_id')}",
            f"hint:       {failure.get('hint')}",
            f"run:        {failure.get('run')}",
            "",
        ]
    )

lines.append("建议动作：")
for idx, item in enumerate(payload["instructions"], start=1):
    lines.append(f"{idx}. {item}")
lines.append("")

open(research_txt, "w", encoding="utf-8").write("\n".join(lines))
PY
}

verify_due() {
  local now_ts="$1"
  local last_ts="${SUPERVISOR_LAST_VERIFY_TS:-0}"
  (( now_ts - last_ts >= VERIFY_INTERVAL_SECONDS ))
}

run_verify_if_due() {
  local hub_url="$1"
  local now_ts="$2"

  if ! verify_due "$now_ts"; then
    return 0
  fi

  if [[ ! -x scripts/verify_mvp.py ]]; then
    add_action "缺少 scripts/verify_mvp.py：无法执行 MVP Gate 验证。"
    SUPERVISOR_LAST_VERIFY_TS="$now_ts"
    save_supervisor_state
    return 1
  fi

  if [[ ! -f "$VERIFY_GATES_FILE" ]]; then
    add_action "缺少 Gate 定义：$VERIFY_GATES_FILE"
    SUPERVISOR_LAST_VERIFY_TS="$now_ts"
    save_supervisor_state
    return 1
  fi

  echo "[$(ts_now)] 运行 MVP Gate 验证（interval=${VERIFY_INTERVAL_SECONDS}s）" >>"$BOOTSTRAP_LOG"
  HUB_URL="$hub_url" scripts/verify_mvp.py \
    --gates "$VERIFY_GATES_FILE" \
    --state-dir "$STATE_DIR" \
    --json-out "$VERIFY_JSON" \
    --text-out "$VERIFY_TEXT" \
    --hub-url "$hub_url" >>"$BOOTSTRAP_LOG" 2>&1 || true

  SUPERVISOR_LAST_VERIFY_TS="$now_ts"

  local parsed=""
  parsed="$(parse_verify_json "$VERIFY_JSON" 2>/dev/null || true)"
  if [[ -n "$parsed" ]]; then
    eval "$parsed"
  fi

  local overall="${VERIFY_OVERALL_STATUS:-}"
  local failure_sig="${VERIFY_FAILURE_SIGNATURE:-}"
  local progress_sig="${VERIFY_PROGRESS_SIGNATURE:-}"

  if [[ -n "$progress_sig" && "$progress_sig" != "${SUPERVISOR_LAST_PROGRESS_SIGNATURE:-}" ]]; then
    add_action "Gate 进度签名发生变化：${SUPERVISOR_LAST_PROGRESS_SIGNATURE:-none} → $progress_sig"
    SUPERVISOR_LAST_PROGRESS_SIGNATURE="$progress_sig"
  fi

  if [[ "$overall" == "pass" ]]; then
    SUPERVISOR_LAST_FAILURE_SIGNATURE=""
    SUPERVISOR_FAILURE_REPEAT_COUNT=0
    clear_research_mode
  elif [[ -n "$failure_sig" ]]; then
    if [[ "$failure_sig" == "${SUPERVISOR_LAST_FAILURE_SIGNATURE:-}" ]]; then
      SUPERVISOR_FAILURE_REPEAT_COUNT=$((SUPERVISOR_FAILURE_REPEAT_COUNT + 1))
    else
      SUPERVISOR_LAST_FAILURE_SIGNATURE="$failure_sig"
      SUPERVISOR_FAILURE_REPEAT_COUNT=1
      # 失败签名变化说明出现了新信息，清理旧的 research mode，避免误导。
      clear_research_mode
    fi

    local since_research=$((now_ts - ${SUPERVISOR_LAST_RESEARCH_TS:-0}))
    if (( SUPERVISOR_FAILURE_REPEAT_COUNT >= RESEARCH_TRIGGER_REPEATS )) && (( since_research >= RESEARCH_COOLDOWN_SECONDS )); then
      if trigger_research_mode "repeat_failure_signature"; then
        SUPERVISOR_LAST_RESEARCH_TS="$now_ts"
        add_action "同一失败签名已重复 ${SUPERVISOR_FAILURE_REPEAT_COUNT} 次：已进入研究模式（$RESEARCH_MODE_TEXT）。"
      else
        add_action "研究模式触发失败：请检查 $VERIFY_JSON"
      fi
    fi
  fi

  SUPERVISOR_LAST_OVERALL_STATUS="${overall:-}"
  SUPERVISOR_LAST_ACTIVE_GATE="${VERIFY_ACTIVE_GATE:-}"
  save_supervisor_state
}

summarize_verify_status() {
  if [[ ! -f "$VERIFY_JSON" ]]; then
    echo "verify: (尚未生成 verify_status.json)"
    return 0
  fi
  python3 - "$VERIFY_JSON" <<'PY'
import json
import sys

path = sys.argv[1]
try:
    data = json.load(open(path, "r", encoding="utf-8"))
except Exception as exc:
    print(f"verify: 读取失败 ({exc})")
    sys.exit(0)

overall = data.get("overall", {}) or {}
gates = data.get("gates", []) or []

generated_at = data.get("generated_at", "?")
status = overall.get("overall_status", "?")
active = overall.get("active_gate_id", "?")
progress = overall.get("progress_signature", "none")
failure_sig = overall.get("failure_signature", "")

print(f"verify: last={generated_at} status={status} active={active} progress={progress}")
if failure_sig:
    print(f"verify: failure_signature={failure_sig}")

if gates:
    parts = []
    for gate in gates:
        gid = gate.get("gate_id", "?")
        gst = gate.get("status", "?")
        parts.append(f"{gid}:{gst}")
    print("gates:  " + ", ".join(parts))

failure = overall.get("failure") or {}
if failure:
    gate_id = failure.get("gate_id", "?")
    check_id = failure.get("check_id", "?")
    hint = failure.get("hint") or ""
    hint = hint[:120] + ("…" if len(hint) > 120 else "")
    print(f"failure: {gate_id}/{check_id} hint={hint}")
PY
}

research_mode_hint() {
  if [[ ! -f "$RESEARCH_MODE_TEXT" ]]; then
    return 0
  fi
  echo "-- RESEARCH MODE --"
  head -n 12 "$RESEARCH_MODE_TEXT" 2>/dev/null || true
  echo
}

check_restart_requested() {
  local now_ts="$1"
  if [[ ! -f "$RESTART_REQUEST_FILE" ]]; then
    return 0
  fi

  # 限流：每小时最多重启 N 次，且两次重启至少间隔冷却时间。
  local window_start="${SUPERVISOR_RESTART_WINDOW_START_TS:-0}"
  local restart_count="${SUPERVISOR_RESTART_COUNT:-0}"
  if (( window_start == 0 || now_ts - window_start >= 3600 )); then
    window_start="$now_ts"
    restart_count=0
  fi

  local since_last=$((now_ts - ${SUPERVISOR_LAST_RESTART_TS:-0}))
  if (( since_last < RESTART_COOLDOWN_SECONDS )); then
    add_action "检测到重启请求但仍在冷却期（${since_last}s < ${RESTART_COOLDOWN_SECONDS}s）：$RESTART_REQUEST_FILE"
    SUPERVISOR_RESTART_WINDOW_START_TS="$window_start"
    SUPERVISOR_RESTART_COUNT="$restart_count"
    save_supervisor_state
    return 0
  fi

  if (( restart_count >= RESTART_LIMIT_PER_HOUR )); then
    add_action "重启次数超过护栏（${RESTART_LIMIT_PER_HOUR}/hour）：请人工检查 $RESTART_REQUEST_FILE"
    SUPERVISOR_RESTART_WINDOW_START_TS="$window_start"
    SUPERVISOR_RESTART_COUNT="$restart_count"
    save_supervisor_state
    return 0
  fi

  local reason=""
  reason="$(head -n 1 "$RESTART_REQUEST_FILE" 2>/dev/null || true)"
  add_action "检测到热重启请求：${reason:-no-reason}（将 exec 监督器）"
  rm -f "$RESTART_REQUEST_FILE"

  restart_count=$((restart_count + 1))
  SUPERVISOR_LAST_RESTART_TS="$now_ts"
  SUPERVISOR_RESTART_WINDOW_START_TS="$window_start"
  SUPERVISOR_RESTART_COUNT="$restart_count"
  # 热重启后尽快重新验证 Gate，避免长时间沿用旧结果。
  SUPERVISOR_LAST_VERIFY_TS=0
  save_supervisor_state

  # 使用 exec 以便接管当前 PID，确保热重启生效。
  exec "$ROOT/scripts/agent_bootstrap.sh"
}

# -------- 上下文摘要模式（用于 agent 循环） --------
context_summary() {
  local hub_url="${HUB:-${HUB_URL:-}}"
  local hub_reachable=0

  echo "== NetMic context =="
  echo "time:   $(ts_now)"
  echo "repo:   $(basename "$ROOT")"
  echo "path:   $ROOT"
  echo "branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'n/a')"
  echo "status:"
  git status -sb || true
  echo

  if [[ -n "$hub_url" ]] && command -v curl >/dev/null 2>&1; then
    echo "-- HUB health --"
    if curl -fsS "$hub_url/v1/health" 2>/dev/null; then
      hub_reachable=1
    else
      echo "unreachable: $hub_url"
    fi
    echo
    if [[ "$hub_reachable" -eq 1 ]]; then
      echo "-- HUB agents (active_within=600) --"
      curl -s "$hub_url/v1/agents?active_within=600&limit=20" || true
      echo
      echo "-- HUB tasks --"
      curl -s "$hub_url/v1/tasks?limit=20" || true
      echo
    fi
  fi

  if [[ "$hub_reachable" -eq 0 ]]; then
    offline_tasks_summary
  fi

  if [[ -f docs/SESSION_LOG.md ]]; then
    echo "-- SESSION_LOG tail --"
    tail -n 15 docs/SESSION_LOG.md
    echo
  fi

  if [[ -f docs/DECISIONS.md ]]; then
    echo "-- DECISIONS tail --"
    tail -n 10 docs/DECISIONS.md
    echo
  fi

  if [[ -f docs/TODO.md ]]; then
    echo "-- TODO head --"
    head -n 20 docs/TODO.md
    echo
  fi

  if command -v gh >/dev/null 2>&1; then
    echo "-- Open issues (gh) --"
    gh issue list -L 10 || true
    echo
  fi

  load_supervisor_state
  if command -v python3 >/dev/null 2>&1; then
    echo "-- MVP GATES --"
    summarize_verify_status || true
    echo
  fi
  research_mode_hint
  if [[ -f "$RESTART_REQUEST_FILE" ]]; then
    echo "-- RESTART REQUEST --"
    head -n 3 "$RESTART_REQUEST_FILE" 2>/dev/null || true
    echo
  fi

  echo "Next: follow MVP.md + docs/ROADMAP.md, pick one small task and validate it."
}

# 在 agent 角色环境中只做上下文摘要，避免递归拉起 autopilot。
if [[ "$CONTEXT_ONLY" == "1" || -n "${ROLE:-}" ]]; then
  context_summary
  exit 0
fi

# -------- 停止/状态模式 --------
if [[ "$STOP_ONLY" == "1" ]]; then
  scripts/autopilot.sh stop || true
  echo "已请求停止 autopilot 与 Hub（若存在）。"
  exit 0
fi

if [[ "$STATUS_ONLY" == "1" ]]; then
  # 仅展示状态，不做启动动作。
  scripts/autopilot.sh status || true
  exit 0
fi

# -------- 前台监督器模式 --------

# 默认启用危险模式，避免被 approvals/sandbox 阻塞；允许用户显式覆盖。
export AUTOPILOT_DANGEROUS="${AUTOPILOT_DANGEROUS:-1}"
# 避免覆盖/污染已跟踪的 .autopilot/hub_url.txt。
export HUB_URL_FILE="$RUNTIME_HUB_URL_FILE"

# shellcheck disable=SC2034
ACTION_ITEMS=()
BLOCK_AUTOPILOT=0

add_action() {
  local msg="$1"
  ACTION_ITEMS+=("$msg")
}

require_cmd_soft() {
  local cmd="$1"
  local hint="$2"
  local critical="${3:-0}"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    add_action "缺少命令 '$cmd'：$hint"
    if [[ "$critical" == "1" ]]; then
      BLOCK_AUTOPILOT=1
    fi
    return 1
  fi
  return 0
}

extract_host() {
  local url="$1"
  local host="${url#http://}"
  host="${host%%/*}"
  host="${host%%:*}"
  printf "%s" "$host"
}

detect_default_ip() {
  local ip=""
  if command -v ip >/dev/null 2>&1; then
    ip="$(ip route get 1.1.1.1 2>/dev/null | awk '/src/ {for (i=1;i<=NF;i++) if ($i=="src") {print $(i+1); exit}}')"
  fi
  if [[ -z "$ip" ]] && [[ "$(uname -s 2>/dev/null || true)" == "Darwin" ]] && command -v ipconfig >/dev/null 2>&1; then
    ip="$(ipconfig getifaddr en0 2>/dev/null || true)"
  fi
  if [[ -z "$ip" ]] && command -v hostname >/dev/null 2>&1; then
    ip="$(hostname -I 2>/dev/null | awk '{print $1}')"
  fi
  if [[ -z "$ip" ]]; then
    ip="127.0.0.1"
  fi
  printf "%s" "$ip"
}

local_ip_list() {
  local ips=""
  if command -v hostname >/dev/null 2>&1; then
    ips="$(hostname -I 2>/dev/null || true)"
  fi
  if [[ -z "$ips" ]] && [[ "$(uname -s 2>/dev/null || true)" == "Darwin" ]] && command -v ipconfig >/dev/null 2>&1; then
    ips="$(ipconfig getifaddr en0 2>/dev/null || true)"
  fi
  if [[ -z "$ips" ]]; then
    ips="$(detect_default_ip)"
  fi
  printf "%s" "$ips"
}

hub_is_local() {
  local host="$1"
  case "$host" in
    127.0.0.1|localhost|0.0.0.0)
      return 0
      ;;
  esac
  local ips
  ips="$(local_ip_list)"
  for ip in $ips; do
    if [[ "$host" == "$ip" ]]; then
      return 0
    fi
  done
  return 1
}

hub_health() {
  local url="$1"
  if ! command -v curl >/dev/null 2>&1; then
    return 1
  fi
  curl -fsS "$url/v1/health" >/dev/null 2>&1
}

load_runner_env() {
  if [[ -f "$RUNNER_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$RUNNER_ENV_FILE"
    return 0
  fi
  if [[ -f "$RUNNER_ENV_EXAMPLE" ]]; then
    add_action "可选：复制 $RUNNER_ENV_EXAMPLE 为 $RUNNER_ENV_FILE，并填写跨机配置（如 BUILDER_MAC_SSH）。"
  fi
  return 0
}

ensure_docs() {
  local today
  today="$(date +%F)"

  if [[ ! -s docs/SESSION_LOG.md ]]; then
    cat <<DOC > docs/SESSION_LOG.md
# SESSION LOG

## $today
- DONE:
- BLOCKER:
- NEXT:
- TEST:
DOC
  fi

  if [[ ! -s docs/DECISIONS.md ]]; then
    cat <<DOC > docs/DECISIONS.md
# DECISIONS

## $today
- 背景：
- 决策：
- 影响：
DOC
  fi

  if [[ ! -s docs/TODO.md ]]; then
    cat <<DOC > docs/TODO.md
# TODO（MVP 自动驾驶看板）

建议将任务拆成小块，并与 Hub 任务状态保持一致。
DOC
  fi
}

resolve_hub_url() {
  load_runner_env

  local hub_url="${HUB_URL:-}"

  if [[ -z "$hub_url" && -f "$HUB_URL_SHARED_FILE" ]]; then
    hub_url="$(head -n 1 "$HUB_URL_SHARED_FILE" | tr -d '[:space:]')"
  fi

  if [[ -z "$hub_url" && -f "$RUNTIME_HUB_URL_FILE" ]]; then
    hub_url="$(head -n 1 "$RUNTIME_HUB_URL_FILE" | tr -d '[:space:]')"
  fi

  # 最后兜底：自动探测本机地址。
  if [[ -z "$hub_url" ]]; then
    local ip
    ip="$(detect_default_ip)"
    hub_url="http://$ip:7788"
  fi

  # 若 hub 不健康且不是本机地址，自动回退到本机。
  local host
  host="$(extract_host "$hub_url")"
  if ! hub_health "$hub_url" && ! hub_is_local "$host"; then
    local ip
    ip="$(detect_default_ip)"
    local fallback="http://$ip:7788"
    add_action "Hub 不健康且非本机地址：$hub_url。已回退为本机 Hub：$fallback"
    hub_url="$fallback"
  fi

  printf "%s\n" "$hub_url" >"$RUNTIME_HUB_URL_FILE"
  export HUB_URL="$hub_url"
}

start_hub_if_needed() {
  local hub_url="$1"
  if hub_health "$hub_url"; then
    return 0
  fi

  local host
  host="$(extract_host "$hub_url")"
  if ! hub_is_local "$host"; then
    add_action "Hub 不可用且不在本机：$hub_url（请确认远端 Hub 已启动）。"
    return 1
  fi

  if ! command -v python3 >/dev/null 2>&1; then
    add_action "无法启动 Hub：缺少 python3。"
    return 1
  fi

  echo "[$(ts_now)] 启动本机 Agent Hub：$hub_url" >>"$BOOTSTRAP_LOG"
  AGENT_HUB_PORT="${hub_url##*:}" python3 agent-hub/agent_hub.py >>"$LOG_DIR/hub.log" 2>&1 &
  echo $! >"$STATE_DIR/hub.pid"
  sleep 1
  if ! hub_health "$hub_url"; then
    # 在受限环境（如沙箱/容器）中，监听端口可能直接被拒绝。
    local hub_log_tail=""
    hub_log_tail="$(tail -n 20 "$LOG_DIR/hub.log" 2>/dev/null || true)"
    if printf "%s" "$hub_log_tail" | grep -qiE "permissionerror|operation not permitted|permission denied"; then
      add_action "Agent Hub 启动失败：当前环境可能禁止监听端口（PermissionError）。可在本机终端运行，或改用远端 Hub。日志：$LOG_DIR/hub.log"
    else
      add_action "Agent Hub 启动失败，请查看 $LOG_DIR/hub.log"
    fi
    return 1
  fi
  return 0
}

codex_auth_ok() {
  if ! command -v codex >/dev/null 2>&1; then
    return 1
  fi
  # 兼容不同版本的 codex CLI：优先 login status，回退到旧的 auth status。
  if command -v timeout >/dev/null 2>&1; then
    timeout 5 codex login status >/dev/null 2>&1 && return 0
    timeout 5 codex auth status >/dev/null 2>&1 && return 0
  else
    codex login status >/dev/null 2>&1 && return 0
    codex auth status >/dev/null 2>&1 && return 0
  fi
  return 1
}

ensure_rust_toolchain() {
  # 先尝试加载 rustup 环境，避免“已安装但 PATH 未生效”的误判。
  # shellcheck disable=SC1090
  if [[ -f "$HOME/.cargo/env" ]]; then
    . "$HOME/.cargo/env"
  fi

  if command -v cargo >/dev/null 2>&1; then
    return 0
  fi

  if [[ "$AUTO_INSTALL_RUST" != "1" ]]; then
    add_action "缺少 cargo：建议安装 rustup（https://rustup.rs），否则无法构建 Rust MVP。"
    BLOCK_AUTOPILOT=1
    return 1
  fi

  require_cmd_soft curl "自动安装 Rust 工具链依赖 curl。" 1 || return 1

  add_action "检测到缺少 cargo：已启用 NETMIC_AUTO_INSTALL_RUST=1，将尝试通过 rustup 自动安装（profile=$RUSTUP_PROFILE）。"

  # rustup 官方一键安装（非交互）。
  if curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile "$RUSTUP_PROFILE" >>"$BOOTSTRAP_LOG" 2>&1; then
    # shellcheck disable=SC1090
    if [[ -f "$HOME/.cargo/env" ]]; then
      . "$HOME/.cargo/env"
    fi
  else
    add_action "rustup 自动安装失败：请查看 $BOOTSTRAP_LOG"
    BLOCK_AUTOPILOT=1
    return 1
  fi

  if command -v cargo >/dev/null 2>&1; then
    add_action "rustup 自动安装完成：cargo 已可用。"
    return 0
  fi

  add_action "rustup 安装似乎完成但 cargo 仍不可用：请查看 $BOOTSTRAP_LOG，并确认 $HOME/.cargo/env 已生效。"
  BLOCK_AUTOPILOT=1
  return 1
}

preflight_checks() {
  ACTION_ITEMS=()
  BLOCK_AUTOPILOT=0

  require_cmd_soft git "请安装 git 并确保当前仓库可读写。" 1 || true
  require_cmd_soft curl "用于 Hub 健康检查与状态轮询。" 1 || true
  require_cmd_soft python3 "用于 Agent Hub 与种子脚本。" 1 || true
  require_cmd_soft codex "用于自动开发与提交（codex CLI）。" 1 || true

  ensure_rust_toolchain || true

  if [[ "$(uname -s 2>/dev/null || true)" == "Linux" ]]; then
    if ! command -v pactl >/dev/null 2>&1; then
      add_action "Linux 缺少 pactl：MVP 需要 PipeWire/Pulse 兼容层（pipewire-pulse 或 pulseaudio）。"
    fi
  fi

  if [[ "$(uname -s 2>/dev/null || true)" != "Darwin" && -z "${BUILDER_MAC_SSH:-}" ]]; then
    add_action "未配置 BUILDER_MAC_SSH：若需要自动驱动 mac builder，请在 $RUNNER_ENV_FILE 中配置 SSH 信息，或在 mac 机器也运行本脚本。"
  fi

  if command -v codex >/dev/null 2>&1 && ! codex_auth_ok; then
    add_action "codex 可能尚未登录：请在本机先运行 'codex login'（旧版可用 'codex auth login'）。"
    BLOCK_AUTOPILOT=1
  fi
}

ensure_workspace_bootstrap() {
  if [[ -f "$WORKSPACE_MARKER" ]]; then
    return 0
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    return 1
  fi
  if ! scripts/bootstrap/bootstrap_workspace.sh >>"$BOOTSTRAP_LOG" 2>&1; then
    add_action "Rust 工作区骨架初始化失败：请查看 $BOOTSTRAP_LOG"
    return 1
  fi
  if ! cargo check >>"$BOOTSTRAP_LOG" 2>&1; then
    add_action "cargo check 失败：请查看 $BOOTSTRAP_LOG（可能需要补依赖或修复编译错误）。"
    return 1
  fi
  echo "$(ts_now)" >"$WORKSPACE_MARKER"
  return 0
}

seed_tasks_if_possible() {
  local hub_url="$1"
  if ! hub_health "$hub_url"; then
    return 1
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    return 1
  fi
  scripts/seed_tasks.py --hub "$hub_url" >>"$BOOTSTRAP_LOG" 2>&1 || add_action "任务种子注入失败：请查看 $BOOTSTRAP_LOG"
}

autopilot_running() {
  local roles=(orchestrator builder-linux scribe)
  local running=0
  for role in "${roles[@]}"; do
    local pid_file="$STATE_DIR/$role.pid"
    if [[ -f "$pid_file" ]] && ps -p "$(cat "$pid_file")" >/dev/null 2>&1; then
      running=$((running + 1))
    fi
  done
  [[ "$running" -ge 2 ]]
}

start_autopilot_if_needed() {
  local hub_url="$1"
  if autopilot_running; then
    return 0
  fi
  if [[ "$BLOCK_AUTOPILOT" == "1" ]]; then
    add_action "检测到关键阻塞项：暂不自动启动 autopilot。请先解决置顶事项后重跑本脚本。"
    return 1
  fi
  if ! command -v codex >/dev/null 2>&1; then
    return 1
  fi
  echo "[$(ts_now)] 启动 autopilot（hub=$hub_url）" >>"$BOOTSTRAP_LOG"
  HUB_URL="$hub_url" scripts/autopilot.sh start >>"$BOOTSTRAP_LOG" 2>&1 || add_action "autopilot 启动失败：请查看 $BOOTSTRAP_LOG"
}

summarize_hub_status() {
  local hub_url="$1"
  if ! command -v curl >/dev/null 2>&1; then
    echo "hub: unknown（缺少 curl）"
    return 0
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "hub: unknown（缺少 python3）"
    return 0
  fi
  if ! hub_health "$hub_url"; then
    echo "hub: unhealthy ($hub_url)"
    return 0
  fi

  local health_json agents_json tasks_json
  health_json="$(curl -fsS "$hub_url/v1/health" 2>/dev/null || true)"
  agents_json="$(curl -fsS "$hub_url/v1/agents?active_within=900&limit=50" 2>/dev/null || true)"
  tasks_json="$(curl -fsS "$hub_url/v1/tasks?limit=50" 2>/dev/null || true)"

  HEALTH_JSON="$health_json" AGENTS_JSON="$agents_json" TASKS_JSON="$tasks_json" python3 - <<'PY'
import collections
import json
import os

def load(raw_text):
    if not raw_text:
        return {}
    try:
        return json.loads(raw_text)
    except Exception:
        return {}

health = load(os.environ.get("HEALTH_JSON", ""))
agents = load(os.environ.get("AGENTS_JSON", "")).get("agents", [])
tasks = load(os.environ.get("TASKS_JSON", "")).get("tasks", [])

print(f"hub: ok agents={len(agents)} tasks={len(tasks)}")

if agents:
    roles = collections.Counter(a.get("role") or "unknown" for a in agents)
    role_parts = ", ".join(f"{k}:{v}" for k, v in sorted(roles.items()))
    print(f"agents: {role_parts}")
else:
    print("agents: (none active in last 900s)")

if tasks:
    status_counts = collections.Counter(t.get("status") or "unknown" for t in tasks)
    status_parts = ", ".join(f"{k}:{v}" for k, v in sorted(status_counts.items()))
    print(f"tasks: {status_parts}")
    top = tasks[:5]
    for task in top:
        tid = task.get("task_id", "?")
        status = task.get("status", "?")
        claimed_by = task.get("claimed_by")
        summary = (task.get("summary") or "").strip()
        summary = summary[:60] + ("…" if len(summary) > 60 else "")
        claimed_part = f" claimed_by={claimed_by}" if claimed_by else ""
        summary_part = f" summary={summary}" if summary else ""
        print(f"  - {tid} [{status}]{claimed_part}{summary_part}")
else:
    print("tasks: (none)")
PY
}

role_pid_line() {
  local role="$1"
  local hub_url="$2"
  local pid_file="$STATE_DIR/$role.pid"
  if [[ "$role" == "hub" ]]; then
    if hub_health "$hub_url"; then
      if [[ -f "$pid_file" ]] && ps -p "$(cat "$pid_file")" >/dev/null 2>&1; then
        echo "hub: running pid=$(cat "$pid_file")"
      else
        echo "hub: healthy（pid 未知，可能由外部启动）"
      fi
      return 0
    fi
    echo "hub: not running"
    return 0
  fi

  if [[ -f "$pid_file" ]] && ps -p "$(cat "$pid_file")" >/dev/null 2>&1; then
    echo "$role: running pid=$(cat "$pid_file")"
  else
    echo "$role: not running"
  fi
}

show_supervisor_screen() {
  local hub_url="$1"

  printf '\033[2J\033[H'
  echo "== NetMic 自动驾驶监督器 =="
  echo "time:   $(ts_now)"
  echo "repo:   $(basename "$ROOT")"
  echo "branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'n/a')"
  echo "hub:    $hub_url"
  echo "mode:   foreground supervisor (Ctrl+C 退出；不会自动 stop)"
  echo

  if [[ "${#ACTION_ITEMS[@]}" -gt 0 ]]; then
    echo "!! 需要用户处理的事项（置顶提示）"
    local idx=1
    for item in "${ACTION_ITEMS[@]}"; do
      echo "${idx}. $item"
      idx=$((idx + 1))
    done
    echo
  else
    echo "当前未检测到阻塞项；自动驾驶应在后台持续推进。"
    echo
  fi

  echo "-- Hub 概览 --"
  summarize_hub_status "$hub_url" || true
  echo

  echo "-- MVP GATE 验证 --"
  if command -v python3 >/dev/null 2>&1; then
    summarize_verify_status || true
  else
    echo "verify: unknown（缺少 python3）"
  fi
  echo

  research_mode_hint
  if [[ -f "$RESTART_REQUEST_FILE" ]]; then
    echo "-- RESTART REQUEST --"
    head -n 3 "$RESTART_REQUEST_FILE" 2>/dev/null || true
    echo
  fi

  echo "-- 进程状态 --"
  role_pid_line hub "$hub_url"
  role_pid_line orchestrator "$hub_url"
  role_pid_line builder-linux "$hub_url"
  role_pid_line scribe "$hub_url"
  echo "builder-mac: 由 autopilot 通过 SSH 启动（若配置 BUILDER_MAC_SSH）"
  echo

  echo "-- SESSION_LOG tail --"
  tail -n 12 docs/SESSION_LOG.md 2>/dev/null || echo "SESSION_LOG 不存在"
  echo

  echo "日志位置：$LOG_DIR"
  echo "监督日志：$BOOTSTRAP_LOG"
}

main_loop() {
  ensure_docs
  resolve_hub_url
  load_supervisor_state

  local last_seed_ts=0

  while true; do
    preflight_checks

    resolve_hub_url
    local hub_url="$HUB_URL"

    local restart_check_ts
    restart_check_ts="$(date +%s)"
    check_restart_requested "$restart_check_ts"

    start_hub_if_needed "$hub_url" || true

    if [[ -z "${HUB_URL:-}" ]]; then
      add_action "无法解析 HUB_URL，自动驾驶无法继续。"
    fi

    # 仅在 cargo 可用时尝试一次骨架初始化。
    ensure_workspace_bootstrap || true

    local now_ts
    now_ts="$(date +%s)"
    run_verify_if_due "$hub_url" "$now_ts" || true

    # 每 5 分钟尝试一次任务种子注入（Hub healthy 前提）。
    if (( now_ts - last_seed_ts >= 300 )); then
      seed_tasks_if_possible "$hub_url" || true
      last_seed_ts=$now_ts
    fi

    start_autopilot_if_needed "$hub_url" || true

    show_supervisor_screen "$hub_url"

    if [[ "$ONCE" == "1" ]]; then
      break
    fi

    sleep "$REFRESH_SECONDS"
  done
}

main_loop
