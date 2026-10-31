#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/capture-caller-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require capture-caller --scope capture-caller
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-capture-caller.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_capture_caller_components.py formal --output "$EVIDENCE/reproduction"
