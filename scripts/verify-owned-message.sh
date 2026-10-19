#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/owned-message-inputs.sha256
bash scripts/python-entry.sh tools/check_owned_message_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-owned-message.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/owned-message/preflight.py --output "$EVIDENCE/frontend"
lake build SandboxModel.Boundaries.OwnedMessage.Audit
lake env lean SandboxModel/Boundaries/OwnedMessage/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require owned-message
echo 'Owned-message conditional model and original-source native positive/negative obligations passed.'
echo 'No claim of Aeneas translation or machine refinement of the async Guest wiring.'
