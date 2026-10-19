#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
step="$1"
shift
if [[ -n "${SANDPROOF_FORMAL_RUNTIME:-}" ]]; then
  bash "$ROOT_DIR/scripts/python-entry.sh" "$ROOT_DIR/tools/proof_batch.py" step "$step" -- "$@"
else
  # Standalone entry: recheck normally, never silently skip a dependency.
  "$@"
fi
