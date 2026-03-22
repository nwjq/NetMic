#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT_DIR/apps/netmic-ui"
BUNDLE_CONFIG="src-tauri/tauri.bundle.conf.json"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo 未安装或不在 PATH 中" >&2
  exit 1
fi

if ! cargo tauri -V >/dev/null 2>&1; then
  echo "error: tauri-cli 未安装；请先执行 cargo install tauri-cli" >&2
  exit 1
fi

platform="$(uname -s)"
case "$platform" in
  Darwin)
    bundles="app,dmg"
    artifact_dirs=(
      "$ROOT_DIR/target/release/bundle/macos"
      "$ROOT_DIR/target/release/bundle/dmg"
    )
    ;;
  Linux)
    bundles="deb"
    artifact_dirs=(
      "$ROOT_DIR/target/release/bundle/deb"
    )
    ;;
  *)
    echo "error: 当前仅支持 macOS(Darwin) 和 Linux，检测到 $platform" >&2
    exit 1
    ;;
esac

echo "==> Packaging NetMic for $platform"
echo "==> Bundles: $bundles"

(
  cd "$APP_DIR"
  cargo tauri build --bundles "$bundles" --no-sign -c "$BUNDLE_CONFIG" "$@"
)

echo
echo "==> Artifacts"
for dir in "${artifact_dirs[@]}"; do
  if [ -d "$dir" ]; then
    find "$dir" -maxdepth 1 -mindepth 1 | sort
  fi
done
