#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/callback-state-inputs.sha256
bash scripts/python-entry.sh tools/check_callback_state_contract.py
# Shared registration is owned by the integration coordinator. Missing, stale
# or unsealed native evidence must fail; this is not an optional test skip.
bash scripts/python-entry.sh -c 'from tools.acceptance_receipts import ROOT, native_case; native_case("callback-state", ROOT, None)'
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-callback-state.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/callback-state/replay.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" \
  --output "$EVIDENCE/reproduction"
echo 'Actual callback helper formal checks and sealed native component passed.'
echo 'A6 normal traversal/B4 mechanical external-state binding remain explicit; no general FnMut or panic heap refinement.'
