#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/joinset-inputs.sha256
bash scripts/python-entry.sh tools/check_joinset_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-joinset.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/joinset/preflight.py --output "$EVIDENCE/frontend"
lake build SandboxModel.Boundaries.JoinSet.Audit
lake env lean SandboxModel/Boundaries/JoinSet/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require joinset
echo 'JoinSet conditional models and original-source native obligations passed.'
echo 'No claim of async Rust machine refinement, physical resource exit or unconditional progress.'
