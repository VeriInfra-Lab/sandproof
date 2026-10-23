#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/tokio-rwlock-inputs.sha256
bash scripts/python-entry.sh tools/check_tokio_rwlock_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-tokio-rwlock.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/tokio-rwlock/preflight.py --output "$EVIDENCE/original-preflight"
lake build SandboxModel.Boundaries.TokioRwLock.Audit
lake env lean SandboxModel/Boundaries/TokioRwLock/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require tokio-rwlock
echo 'Conditional Tokio RwLock read/write/map rules and original-lock bindings checked.'
echo 'A6/B2 source wiring remains trusted; authentication and full async Rust refinement are not claimed.'
