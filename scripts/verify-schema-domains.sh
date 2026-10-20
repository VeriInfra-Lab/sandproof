#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/schema-domains-inputs.sha256
bash scripts/python-entry.sh tools/check_schema_domains.py
bash scripts/python-entry.sh tools/check_schema_domains_binding.py
# The integration owner must register and seal this native component first.
bash scripts/python-entry.sh -c 'from tools.acceptance_receipts import ROOT, native_case; native_case("schema-domains", ROOT, None)'
bash scripts/python-entry.sh tools/check_foundations_extraction.py --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-schema-domains.XXXXXXXX")"
bash scripts/python-entry.sh verification/experiments/schema-domains/replay.py --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" --output "$EVIDENCE/reproduction"
echo 'Status guard, ordinary SliceRead/Vec binding and actual Rust counterexamples passed.'
echo 'Conditional A6/B4; server bounded Writer and whole async service remain unproved.'
