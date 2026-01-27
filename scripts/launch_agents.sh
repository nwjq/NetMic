#!/usr/bin/env bash
# Create a tmux session with panes for hub and role agents.
set -euo pipefail

SESSION="${SESSION:-netmic-agents}"
HUB_URL="${HUB_URL:-http://127.0.0.1:7788}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v tmux >/dev/null 2>&1; then
  echo "tmux is required. Install tmux or start agents manually." >&2
  exit 1
fi

tmux has-session -t "$SESSION" 2>/dev/null && {
  echo "tmux session '$SESSION' already exists. Attach with: tmux attach -t $SESSION"
  exit 0
}

# Window 0: Agent Hub (Linux runner recommended)
tmux new-session -d -s "$SESSION" -n hub "cd \"$ROOT\" && HUB_URL=\"$HUB_URL\" python3 agent-hub/agent_hub.py"

# Window 1: orchestrator
tmux new-window -t "$SESSION" -n orchestrator "cd \"$ROOT\" && export HUB=\"$HUB_URL\" ROLE=orchestrator; bash"

# Window 2: builder-linux
tmux new-window -t "$SESSION" -n builder-linux "cd \"$ROOT\" && export HUB=\"$HUB_URL\" ROLE=builder-linux; bash"

# Window 3: builder-mac (start this window on the mac runner if using ssh)
tmux new-window -t "$SESSION" -n builder-mac "echo 'Run on mac runner: HUB=$HUB_URL ROLE=builder-mac codex'; bash"

# Window 4: scribe
tmux new-window -t "$SESSION" -n scribe "cd \"$ROOT\" && export HUB=\"$HUB_URL\" ROLE=scribe; bash"

tmux select-window -t "$SESSION:hub"
echo "Created tmux session '$SESSION'. Attach with:"
echo "  tmux attach -t $SESSION"
