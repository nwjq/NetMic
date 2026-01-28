#!/usr/bin/env bash
set -euo pipefail
cd "/home/arc/code/NetMic"
export HUB="http://127.0.0.1:7788" ROLE="scribe" SESSION_LABEL="autopilot"

cleanup_children() {
  local children
  children="$(ps -o pid= --ppid $$ 2>/dev/null || true)"
  if [[ -n "${children// }" ]]; then
    kill $children >/dev/null 2>&1 || true
  fi
}
trap cleanup_children EXIT INT TERM

iter=0
while true; do
  iter=$((iter+1))
  echo "[$(date -Iseconds)] role=scribe iter=$iter" >>"/home/arc/code/NetMic/.autopilot/logs/scribe.log"
  BOOTSTRAP_CONTEXT_ONLY=1 scripts/agent_bootstrap.sh --context >>"/home/arc/code/NetMic/.autopilot/logs/scribe.log" 2>&1 || true
  codex exec --full-auto -C /home/arc/code/NetMic -o "/home/arc/code/NetMic/.autopilot/logs/scribe.last.txt" --json <"/home/arc/code/NetMic/.autopilot/scribe.prompt.txt" >>"/home/arc/code/NetMic/.autopilot/logs/scribe.log" 2>&1 || true
  if [[ "0" -gt 0 && "$iter" -ge "0" ]]; then
    exit 0
  fi
  sleep "300"
done
