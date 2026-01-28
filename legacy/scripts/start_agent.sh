#!/usr/bin/env bash
# Start a Codex agent with ROLE and HUB exported.
set -euo pipefail

ROLE="${ROLE:-}"
HUB="${HUB:-}"

if [[ -z "$ROLE" ]]; then
  echo "ROLE is required (orchestrator|builder-linux|builder-mac|scribe)" >&2
  exit 1
fi

if [[ -z "$HUB" ]]; then
  echo "HUB is required (e.g. http://192.168.1.10:7788)" >&2
  exit 1
fi

export ROLE HUB
exec codex
