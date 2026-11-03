#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/pause-dispatch-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require pause-dispatch --scope pause-dispatch
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-pause-dispatch.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_pause_dispatch_components.py formal --output "$EVIDENCE/reproduction"
