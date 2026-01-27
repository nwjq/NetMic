#!/usr/bin/env bash
# One-command multi-agent autopilot launcher and supervisor.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="$ROOT/.autopilot"
LOG_DIR="$STATE_DIR/logs"

HUB_PORT="${HUB_PORT:-7788}"
HUB_URL_FILE="${HUB_URL_FILE:-$STATE_DIR/hub_url.txt}"
HUB_URL_SHARED_FILE="${HUB_URL_SHARED_FILE:-$ROOT/docs/HUB_URL.txt}"
HUB_URL="${HUB_URL:-}"
LOOP_SLEEP_SECONDS="${LOOP_SLEEP_SECONDS:-180}"
AGENT_ITERATIONS="${AGENT_ITERATIONS:-0}" # 0 means infinite loop.
SESSION_LABEL="${SESSION_LABEL:-autopilot}"

CODEX_FLAGS=(--full-auto -C "$ROOT")
if [[ "${AUTOPILOT_DANGEROUS:-0}" == "1" ]]; then
  CODEX_FLAGS+=(--dangerously-bypass-approvals-and-sandbox)
fi

usage() {
  cat <<EOF
Usage: scripts/autopilot.sh <start|stop|status>

Env overrides:
  HUB_URL=http://<hub-ip>:7788
  HUB_PORT=7788
  HUB_URL_SHARED_FILE=docs/HUB_URL.txt
  LOOP_SLEEP_SECONDS=180
  AGENT_ITERATIONS=0
  AUTOPILOT_DANGEROUS=1
  BUILDER_MAC_SSH=user@mac-host   # optional; starts builder-mac via ssh
  BUILDER_MAC_ROOT=/path/to/NetMic
EOF
}

require_cmds() {
  local missing=0
  for cmd in curl python3 codex; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      echo "missing required command: $cmd" >&2
      missing=1
    fi
  done
  if [[ "$missing" -ne 0 ]]; then
    exit 1
  fi
}

extract_host() {
  local url="$1"
  local host="${url#http://}"
  host="${host%%/*}"
  host="${host%%:*}"
  printf "%s" "$host"
}

extract_port() {
  local url="$1"
  local hostport="${url#http://}"
  hostport="${hostport%%/*}"
  if [[ "$hostport" == *:* ]]; then
    printf "%s" "${hostport##*:}"
    return 0
  fi
  printf "%s" "$HUB_PORT"
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

resolve_hub_url() {
  mkdir -p "$STATE_DIR" "$LOG_DIR"
  if [[ -n "$HUB_URL" ]]; then
    printf "%s\n" "$HUB_URL" >"$HUB_URL_FILE"
    return 0
  fi
  if [[ -f "$HUB_URL_FILE" ]]; then
    HUB_URL="$(head -n 1 "$HUB_URL_FILE" | tr -d '[:space:]')"
    if [[ -n "$HUB_URL" ]]; then
      return 0
    fi
  fi
  if [[ -f "$HUB_URL_SHARED_FILE" ]]; then
    HUB_URL="$(head -n 1 "$HUB_URL_SHARED_FILE" | tr -d '[:space:]')"
    if [[ -n "$HUB_URL" ]]; then
      printf "%s\n" "$HUB_URL" >"$HUB_URL_FILE"
      return 0
    fi
  fi
  local detected_ip
  detected_ip="$(detect_default_ip)"
  HUB_URL="http://$detected_ip:$HUB_PORT"
  printf "%s\n" "$HUB_URL" >"$HUB_URL_FILE"
}

hub_is_local() {
  local host
  host="$(extract_host "$HUB_URL")"
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
  curl -fsS "$HUB_URL/v1/health" >/dev/null 2>&1
}

start_hub_if_needed() {
  mkdir -p "$LOG_DIR"
  if hub_health; then
    return 0
  fi
  if ! hub_is_local; then
    echo "Hub at $HUB_URL is not healthy and not local; refusing to auto-start." >&2
    exit 1
  fi
  echo "Starting local Agent Hub at $HUB_URL"
  local port
  port="$(extract_port "$HUB_URL")"
  (
    cd "$ROOT"
    AGENT_HUB_PORT="$port" python3 agent-hub/agent_hub.py >>"$LOG_DIR/hub.log" 2>&1
  ) &
  echo $! >"$STATE_DIR/hub.pid"
  # Give the hub a moment to come up.
  sleep 1
  hub_health || {
    echo "Agent Hub failed to start; check $LOG_DIR/hub.log" >&2
    exit 1
  }
}

render_prompt() {
  local template="$1"
  local out="$2"
  sed "s|{{HUB_URL}}|$HUB_URL|g" "$template" >"$out"
}

agent_loop_script() {
  local role="$1"
  local prompt_file="$2"
  local loop_file="$STATE_DIR/${role}.loop.sh"
  local pid_file="$STATE_DIR/${role}.pid"
  local log_file="$LOG_DIR/${role}.log"
  local last_file="$LOG_DIR/${role}.last.txt"

  cat >"$loop_file" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "$ROOT"
export HUB="$HUB_URL" ROLE="$role" SESSION_LABEL="$SESSION_LABEL"
iter=0
while true; do
  iter=\$((iter+1))
  echo "[\$(date -Iseconds)] role=$role iter=\$iter" >>"$log_file"
  scripts/agent_bootstrap.sh >>"$log_file" 2>&1 || true
  codex exec ${CODEX_FLAGS[*]} -o "$last_file" --json <"$prompt_file" >>"$log_file" 2>&1 || true
  if [[ "$AGENT_ITERATIONS" -gt 0 && "\$iter" -ge "$AGENT_ITERATIONS" ]]; then
    exit 0
  fi
  sleep "$LOOP_SLEEP_SECONDS"
done
EOF
  chmod +x "$loop_file"

  if [[ -f "$pid_file" ]] && ps -p "$(cat "$pid_file")" >/dev/null 2>&1; then
    echo "role=$role already running (pid $(cat "$pid_file"))"
    return 0
  fi

  echo "Starting role=$role"
  "$loop_file" >>"$log_file" 2>&1 &
  echo $! >"$pid_file"
}

start_agents_local() {
  mkdir -p "$STATE_DIR" "$LOG_DIR"
  # Ensure base docs exist for low-context restarts.
  touch "$ROOT/docs/SESSION_LOG.md" "$ROOT/docs/DECISIONS.md" "$ROOT/docs/TODO.md"

  render_prompt "$ROOT/scripts/prompts/orchestrator.txt" "$STATE_DIR/orchestrator.prompt.txt"
  render_prompt "$ROOT/scripts/prompts/builder_linux.txt" "$STATE_DIR/builder-linux.prompt.txt"
  render_prompt "$ROOT/scripts/prompts/scribe.txt" "$STATE_DIR/scribe.prompt.txt"

  agent_loop_script "orchestrator" "$STATE_DIR/orchestrator.prompt.txt"
  agent_loop_script "builder-linux" "$STATE_DIR/builder-linux.prompt.txt"
  agent_loop_script "scribe" "$STATE_DIR/scribe.prompt.txt"
}

start_builder_mac_remote() {
  local remote_root="${BUILDER_MAC_ROOT:-$ROOT}"
  if [[ -z "${BUILDER_MAC_SSH:-}" ]]; then
    echo "BUILDER_MAC_SSH not set; skipping remote builder-mac start."
    echo "On mac runner, run:"
    echo "  HUB_URL=$HUB_URL ROLE=builder-mac scripts/start_agent.sh"
    return 0
  fi
  echo "Starting builder-mac remotely via ssh: $BUILDER_MAC_SSH"
  ssh "$BUILDER_MAC_SSH" "cd \"$remote_root\" && HUB=\"$HUB_URL\" ROLE=builder-mac scripts/start_agent.sh" || {
    echo "remote builder-mac start failed; ensure repo path and codex auth exist on mac." >&2
  }
}

stop_pid_file() {
  local name="$1"
  local pid_file="$STATE_DIR/$name.pid"
  if [[ ! -f "$pid_file" ]]; then
    return 0
  fi
  local pid
  pid="$(cat "$pid_file" 2>/dev/null || echo "")"
  if [[ -n "$pid" ]] && ps -p "$pid" >/dev/null 2>&1; then
    echo "Stopping $name (pid $pid)"
    kill "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$pid_file"
}

status() {
  resolve_hub_url
  echo "== autopilot status =="
  echo "root: $ROOT"
  echo "hub:  $HUB_URL"
  echo
  if hub_health; then
    echo "-- hub health --"
    curl -s "$HUB_URL/v1/health" || true
    echo
    echo "-- agents --"
    curl -s "$HUB_URL/v1/agents?active_within=900&limit=50" || true
    echo
    echo "-- tasks --"
    curl -s "$HUB_URL/v1/tasks?limit=50" || true
    echo
  else
    echo "hub not healthy at $HUB_URL"
    echo
  fi

  for role in hub orchestrator builder-linux scribe; do
    local pid_file="$STATE_DIR/$role.pid"
    if [[ -f "$pid_file" ]] && ps -p "$(cat "$pid_file")" >/dev/null 2>&1; then
      echo "role=$role pid=$(cat "$pid_file") running"
    else
      echo "role=$role not running"
    fi
  done
  echo
  echo "logs: $LOG_DIR"
}

start() {
  require_cmds
  resolve_hub_url
  start_hub_if_needed
  start_agents_local
  start_builder_mac_remote
  status
}

stop() {
  stop_pid_file "orchestrator"
  stop_pid_file "builder-linux"
  stop_pid_file "scribe"
  stop_pid_file "hub"
}

main() {
  local cmd="${1:-}"
  case "$cmd" in
    start)
      start
      ;;
    stop)
      stop
      ;;
    status)
      status
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "${1:-}"
