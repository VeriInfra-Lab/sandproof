#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/revocation-contract-inputs.sha256
bash scripts/python-entry.sh tools/check_revocation_contract.py
lake build SandboxModel.Boundaries.Revocation.Audit
lake env lean SandboxModel/Boundaries/Revocation/Audit.lean
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-revocation-contract.XXXXXXXX")"
echo "B3 component evidence: $EVIDENCE/reproduction"
bash scripts/python-entry.sh tools/check_revocation_components.py --output "$EVIDENCE/reproduction"
echo 'B3 revocation-wait model, kernel audit and actual Rust component checks passed.'
echo 'Async wiring remains a reviewed trusted adapter; no Aeneas extraction or physical-exit claim.'
