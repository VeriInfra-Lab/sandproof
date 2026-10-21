#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/btree-set-binding-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require btree-set-binding
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-btree-set-binding.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_btree_set_binding.py
bash scripts/python-entry.sh verification/experiments/btree-set-binding/model/replay.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" --output "$EVIDENCE/reproduction"
