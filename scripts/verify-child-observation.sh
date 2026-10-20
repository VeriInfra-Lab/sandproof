#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
export PYTHONPATH="$ROOT_DIR${PYTHONPATH:+:$PYTHONPATH}"
shasum -a 256 -c verification/child-observation-inputs.sha256
bash scripts/python-entry.sh tools/check_child_observation.py
bash scripts/python-entry.sh tools/proof_native_suite.py require child-observation
: "${SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE:?frozen original child-observation archive required}"
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-child-observation.XXXXXXXX")"
bash scripts/regenerate-child-observation.sh \
  --archive "$SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE" \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" --output "$EVIDENCE/extraction"
bash scripts/python-entry.sh verification/experiments/child-observation/proof.py --output "$EVIDENCE/proof"
bash scripts/python-entry.sh tools/check_child_observation.py --formal "$EVIDENCE/proof"
echo 'Actual child core, concrete caller and same-contract wrong Rust checks passed; A2/A3/A6/A8/B4 bindings remain conditional.'
