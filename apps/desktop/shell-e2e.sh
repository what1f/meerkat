#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
BINARY_DIR="${LANKIDE_RELEASE_BIN_DIR:-${CARGO_TARGET_DIR:-$ROOT_DIR/target}/debug}"
if [[ -n "${LANKIDE_VISUAL_BINARY:-}" ]]; then
  BINARY_DIR="$(dirname -- "$LANKIDE_VISUAL_BINARY")"
fi
exec python3 "$ROOT_DIR/apps/desktop/tests/bootstrap_e2e.py" \
  --binary-dir "$BINARY_DIR" \
  --artifact-dir "$ROOT_DIR/target/e2e-artifacts/desktop-bootstrap/${LANKIDE_E2E_RUN_ID:-manual}"
