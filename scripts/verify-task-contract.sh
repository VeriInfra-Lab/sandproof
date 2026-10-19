#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/task-contract-inputs.sha256
bash scripts/python-entry.sh tools/check_task_contract.py
rustfmt --check --edition 2024 --config skip_children=true verification/fixtures/task-contract-tests.rs
lake build SandboxModel.Boundaries.Task.Audit
lake env lean SandboxModel/Boundaries/Task/Audit.lean
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-task-contract.XXXXXXXX")"
echo "B1 component evidence: $EVIDENCE/reproduction"
bash scripts/python-entry.sh tools/check_task_contract_components.py --output "$EVIDENCE/reproduction"
echo 'B1 task/pool interface theorems passed kernel and dependency audits.'
echo 'Actual async wiring remains conditional on the reviewed B1 adapter, not Aeneas-proved.'
