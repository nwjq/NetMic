#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SRC_DIR="$ROOT_DIR/ui"
DIST_DIR="$SRC_DIR/dist"

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

cp "$SRC_DIR/index.html" "$DIST_DIR/index.html"
cp "$SRC_DIR/styles.css" "$DIST_DIR/styles.css"
cp "$SRC_DIR/app.js" "$DIST_DIR/app.js"
cp "$SRC_DIR/app.core.js" "$DIST_DIR/app.core.js"
cp "$SRC_DIR/app.dom.js" "$DIST_DIR/app.dom.js"
cp "$SRC_DIR/app.interactions.js" "$DIST_DIR/app.interactions.js"
