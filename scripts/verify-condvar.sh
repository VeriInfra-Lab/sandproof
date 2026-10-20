#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/condvar-inputs.sha256
bash scripts/python-entry.sh tools/check_condvar_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-condvar.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/condvar/preflight.py --output "$EVIDENCE/original-preflight"
lake build SandboxModel.Boundaries.Condvar.Audit
lake env lean SandboxModel/Boundaries/Condvar/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require condvar
echo 'Conditional Condvar wait/reacquire rules and finite original-object bindings checked.'
echo 'Reaper run/reap/exit/release business remains unproved; A6/B2 wiring is explicit.'
