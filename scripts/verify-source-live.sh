#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/source-live-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require source-live --scope stage1-source-live
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-source-live.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_source_live_components.py formal --output "$EVIDENCE/component"
