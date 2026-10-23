#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/loop-callbacks-inputs.sha256
bash scripts/python-entry.sh tools/check_loop_callbacks_contract.py
# Original receipt admission: this case must be included in the sealed native
# batch by the integration owner before the complete module entry can pass.
bash scripts/python-entry.sh -c 'from tools.acceptance_receipts import ROOT, native_case; native_case("loop-callbacks", ROOT, None)'
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-loop-callbacks.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/loop-callbacks/replay.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" \
  --output "$EVIDENCE/reproduction"
echo 'Original shared-query formal component and sealed native component passed.'
echo 'Lawful comparison and valid value borrows required; no whole-caller, heap snapshot or V0 completion claim.'
