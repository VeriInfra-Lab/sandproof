#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/pause-publication-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require pause-publication --scope pause-publication
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-pause-publication.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_pause_publication_components.py formal --output "$EVIDENCE/reproduction"
