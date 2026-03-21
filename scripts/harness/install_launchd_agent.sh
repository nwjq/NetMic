#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
template_path="$ROOT/.harness/launchd/com.netmic.harness.coordinator.plist.example"
PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH:-}"

label="com.netmic.harness.coordinator"
hosts_env="$ROOT/.harness/hosts.env"
until_target="M3"
interval_sec=1200
target_path="$HOME/Library/LaunchAgents/${label}.plist"
write_only=0

usage() {
  cat <<'EOF'
用法：
  scripts/harness/install_launchd_agent.sh [--hosts-env PATH] [--until M0|M1|M2|M3] [--interval-sec N] [--label LABEL] [--write-only]

说明：
  生成并加载一个 launchd agent，让本机周期执行 NetMic coordinator。
  该 agent 直接运行在当前 Mac 用户会话，不经过 Codex 自动化沙箱，因此可以访问局域网 Linux。
  加上 --write-only 时，只生成并校验 plist，不执行 launchctl。
EOF
}

xml_escape() {
  printf '%s' "$1" | sed \
    -e 's/&/\&amp;/g' \
    -e 's/</\&lt;/g' \
    -e 's/>/\&gt;/g' \
    -e 's/"/\&quot;/g' \
    -e "s/'/\&apos;/g"
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
    --interval-sec)
      interval_sec="$2"
      shift 2
      ;;
    --label)
      label="$2"
      target_path="$HOME/Library/LaunchAgents/${label}.plist"
      shift 2
      ;;
    --write-only)
      write_only=1
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

if [[ "$hosts_env" != /* ]]; then
  hosts_env="$ROOT/$hosts_env"
fi

if [[ ! -f "$template_path" ]]; then
  echo "缺少模板：$template_path" >&2
  exit 1
fi

if [[ ! -f "$hosts_env" ]]; then
  echo "缺少 hosts env：$hosts_env" >&2
  exit 1
fi

if ! [[ "$interval_sec" =~ ^[0-9]+$ ]] || [[ "$interval_sec" -lt 60 ]]; then
  echo "interval-sec 必须是 >= 60 的整数秒：$interval_sec" >&2
  exit 1
fi

mkdir -p "$HOME/Library/LaunchAgents" "$ROOT/.harness/launchd"

stdout_path="$ROOT/.harness/launchd/coordinator.stdout.log"
stderr_path="$ROOT/.harness/launchd/coordinator.stderr.log"
uid="$(id -u)"

sed \
  -e "s|__LABEL__|$(xml_escape "$label")|g" \
  -e "s|__ROOT__|$(xml_escape "$ROOT")|g" \
  -e "s|__HOSTS_ENV__|$(xml_escape "$hosts_env")|g" \
  -e "s|__UNTIL__|$(xml_escape "$until_target")|g" \
  -e "s|__STDOUT_PATH__|$(xml_escape "$stdout_path")|g" \
  -e "s|__STDERR_PATH__|$(xml_escape "$stderr_path")|g" \
  "$template_path" > "$target_path"

/usr/libexec/PlistBuddy -c "Set :StartInterval $interval_sec" "$target_path" >/dev/null
plutil -lint "$target_path" >/dev/null

if [[ "$write_only" -eq 1 ]]; then
  echo "launchd plist 已生成：$target_path"
  exit 0
fi

launchctl bootout "gui/$uid" "$target_path" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$uid" "$target_path"
launchctl enable "gui/$uid/$label" >/dev/null 2>&1 || true
launchctl kickstart -k "gui/$uid/$label"

echo "launchd agent 已安装：$target_path"
echo "stdout: $stdout_path"
echo "stderr: $stderr_path"
