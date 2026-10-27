#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/stage1-protocol-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require stage1-protocol
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-stage1-protocol.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/stage1-protocol/component.py --output "$EVIDENCE/reproduction"
bash scripts/python-entry.sh verification/experiments/stage1-protocol/registered.py formal \
  --retained-result "$EVIDENCE/reproduction/formal/result.json" --output "$EVIDENCE/registered"
echo 'Retained Stage1 protocol profiles and all registered actual Rust mutants passed.'
echo 'Registered synchronous value contracts and explicit library conditions; not whole lifecycle, async, transaction or physical-state correctness.'
