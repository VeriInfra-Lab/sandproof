#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/notify-inputs.sha256
bash scripts/python-entry.sh tools/check_notify_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-notify.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/notify/preflight.py --output "$EVIDENCE/frontend"
lake build SandboxModel.Boundaries.Notify.Audit
lake env lean SandboxModel/Boundaries/Notify/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require notify
echo 'Notify conditional model and original-source native positive/negative obligations passed.'
echo 'No claim of async Rust machine refinement or physical resource exit.'
