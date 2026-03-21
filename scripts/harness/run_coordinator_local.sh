#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

hosts_env="$ROOT/.harness/hosts.env"
until_target="M3"
log_dir="$ROOT/.harness/launchd"
lock_dir="$ROOT/.harness/locks/coordinator.lock"

usage() {
  cat <<'EOF'
用法：
  scripts/harness/run_coordinator_local.sh [--hosts-env PATH] [--until M0|M1|M2|M3] [--log-dir PATH]

说明：
  在当前 Mac 本机直接运行 coordinator，适合被 launchd/cron/tmux 调度。
  脚本会加本地锁，避免 M3 长跑时被下一次调度重入。
EOF
}

timestamp() {
  date '+%Y-%m-%d %H:%M:%S %z'
}

cleanup() {
  rmdir "$lock_dir" 2>/dev/null || true
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --hosts-env)
      hosts_env="$2"
      shift 2
      ;;
    --until)
      until_target="$2"
      shift 2
      ;;
    --log-dir)
      log_dir="$2"
      shift 2
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

if [[ "$hosts_env" != /* ]]; then
  hosts_env="$ROOT/$hosts_env"
fi

if [[ "$log_dir" != /* ]]; then
  log_dir="$ROOT/$log_dir"
fi

mkdir -p "$log_dir" "$(dirname "$lock_dir")"

if [[ ! -f "$hosts_env" ]]; then
  echo "[$(timestamp)] blocked: hosts env 不存在：$hosts_env" >&2
  exit 2
fi

if ! mkdir "$lock_dir" 2>/dev/null; then
  echo "[$(timestamp)] skip: coordinator 仍在运行，跳过本次调度"
  exit 0
fi
trap cleanup EXIT

cd "$ROOT"

echo "[$(timestamp)] start: coordinator --hosts-env $hosts_env --until $until_target"
set +e
python3 scripts/harness/coordinator.py --hosts-env "$hosts_env" --until "$until_target"
status=$?
set -e
echo "[$(timestamp)] finish: exit=$status"
exit "$status"
