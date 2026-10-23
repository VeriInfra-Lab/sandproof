#!/usr/bin/env bash
# Formal-machine module gate. Host supervisor must hold formal-heavy.lock.
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/sqlite-statements-inputs.sha256
# Require the original sealed ning component before running formal checks.
bash scripts/python-entry.sh -c 'from tools.acceptance_receipts import ROOT, native_case; native_case("sqlite-statements", ROOT, None)'
: "${SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE:?Set the frozen production Git archive path}"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
mkdir -p artifacts
BATCH="$(mktemp -d "$ROOT_DIR/artifacts/sqlite-statements-proof.XXXXXXXX")"
echo "SQLite statements formal evidence: $BATCH"
bash scripts/python-entry.sh verification/experiments/sqlite-statements/replay.py \
  --source-archive "$SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE" \
  --output "$BATCH/run" --tool-root "$TOOL_ROOT"
echo 'Matching sealed ning evidence admitted; this formal module passed. Integrated full-suite/CI gates remain required.'
