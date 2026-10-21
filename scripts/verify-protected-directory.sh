#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/protected-directory-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require protected-directory
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-protected-directory.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/protected-directory/check.py
bash scripts/python-entry.sh verification/experiments/protected-directory/proof.py \
  --archive "${SANDPROOF_LIBRARY_SOURCE_ARCHIVE:?required original source archive}" \
  --std-evidence "${SANDPROOF_PROTECTED_DIRECTORY_STD_EVIDENCE:?required reviewed Std files}" \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" --output "$EVIDENCE/reproduction"
