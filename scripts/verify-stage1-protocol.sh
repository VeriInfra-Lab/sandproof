#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
PROFILE=full
if [[ "$#" -eq 2 && "$1" == --profile && "$2" == create-delivery ]]; then
  PROFILE=create-delivery
elif [[ "$#" -ne 0 ]]; then
  echo 'Usage: verify-stage1-protocol.sh [--profile create-delivery]' >&2
  exit 2
fi
if [[ "$PROFILE" == create-delivery ]]; then
  # Deliberately explicit CLI only. Environment variables cannot reduce full/all.
  shasum -a 256 -c verification/create-delivery-inputs.sha256
  bash scripts/python-entry.sh tools/proof_native_suite.py require functional --scope stage1-create-delivery
  EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-create-delivery.XXXXXXXX")"
  bash scripts/python-entry.sh verification/experiments/stage1-protocol/registered.py formal \
    --profile create-delivery --output "$EVIDENCE/registered"
  echo 'All 12 fixed Create packages and 42 actual physical proof views passed.'
  echo 'Finite synchronous Create contracts under registered dependency conditions; not full V3 or system acceptance.'
  exit 0
fi
shasum -a 256 -c verification/stage1-protocol-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require stage1-protocol --scope stage1-protocol
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-stage1-protocol.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/stage1-protocol/component.py --output "$EVIDENCE/reproduction"
bash scripts/python-entry.sh verification/experiments/stage1-protocol/registered.py formal \
  --retained-result "$EVIDENCE/reproduction/formal/result.json" --output "$EVIDENCE/registered"
echo 'Retained Stage1 protocol profiles and all registered actual Rust mutants passed.'
echo 'Registered synchronous value contracts and explicit library conditions; not whole lifecycle, async, transaction or physical-state correctness.'
