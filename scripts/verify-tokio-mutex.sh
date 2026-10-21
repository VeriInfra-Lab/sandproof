#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/tokio-mutex-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require tokio-mutex
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-tokio-mutex.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/tokio-mutex/check.py
bash scripts/python-entry.sh verification/experiments/tokio-mutex/proof.py --output "$EVIDENCE/reproduction"
