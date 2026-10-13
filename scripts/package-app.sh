#!/bin/bash
set -euo pipefail
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo '首版应用包仅支持 macOS Apple Silicon。' >&2; exit 1; }
if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
  CMAKE_POLICY_VERSION_MINIMUM=3.5 "${CARGO:-cargo}" build --release --locked
fi
BINARY="${VOCABULARY_BINARY:-${CARGO_TARGET_DIR:-$PROJECT_DIR/target}/release/zectrix-vocabulary}"
[[ -x "$BINARY" ]] || { echo "未找到可执行程序：$BINARY" >&2; exit 1; }
mkdir -p "$PROJECT_DIR/dist"
STAGE="$(mktemp -d "$PROJECT_DIR/dist/app-stage.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/bin" "$STAGE/workers"
cp "$BINARY" "$STAGE/bin/zectrix-vocabulary"
cp "$PROJECT_DIR"/workers/*.py "$STAGE/workers/"
for ITEM in assets README.md LICENSE THIRD_PARTY_NOTICES.md app.json; do
  cp -R "$PROJECT_DIR/$ITEM" "$STAGE/$ITEM"
done
"${PYTHON:-python3}" "$PROJECT_DIR/scripts/package-app.py" "$STAGE" "$PROJECT_DIR/dist"
