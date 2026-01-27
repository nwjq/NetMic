#!/usr/bin/env bash
# Quick context loader for Codex agents.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== NetMic bootstrap =="
echo "repo:  $(basename "$ROOT")"
echo "path:  $ROOT"
echo "branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'n/a')"
echo "status:"
git status -sb || true
echo

if [[ -n "${HUB:-}" ]] && command -v curl >/dev/null 2>&1; then
  echo "-- HUB health --"
  curl -s "$HUB/v1/health" || true
  echo
  echo "-- HUB agents (active_within=600) --"
  curl -s "$HUB/v1/agents?active_within=600&limit=20" || true
  echo
  echo "-- HUB tasks --"
  curl -s "$HUB/v1/tasks?limit=20" || true
  echo
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

echo "Next: pick an Issue/PR and start the role agent with HUB and ROLE exported."
