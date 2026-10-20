#!/usr/bin/env bash
# Generate evidence only; never overwrite promoted LLBC/Lean or production.
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
: "${SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE:?Set the frozen production Git archive path}"
: "${SANDPROOF_SQLITE_STATEMENTS_GENERATION:?Set a new evidence destination}"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
bash scripts/python-entry.sh verification/experiments/sqlite-statements/prepare.py \
  --source-archive "$SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE" \
  --destination "$SANDPROOF_SQLITE_STATEMENTS_GENERATION/source"
bash scripts/python-entry.sh verification/experiments/sqlite-statements/translate.py \
  --source "$SANDPROOF_SQLITE_STATEMENTS_GENERATION/source" \
  --output "$SANDPROOF_SQLITE_STATEMENTS_GENERATION/extraction" --tool-root "$TOOL_ROOT"
bash scripts/python-entry.sh tools/check_sqlite_statements.py \
  --correct "$SANDPROOF_SQLITE_STATEMENTS_GENERATION/extraction" --tool-root "$TOOL_ROOT"
