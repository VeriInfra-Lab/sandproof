#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/pause-receipt-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require pause-receipt --scope pause-receipt
: "${SANDPROOF_FORMAL_RUNTIME:?original formal batch runtime is required}"
: "${SANDPROOF_FORMAL_CACHE:?original formal module cache is required}"
# Each affected old scope retains its own receipt, counterexamples and source
# checks. The existing step mechanism admits RUN/REUSE; no labels are rewritten.
for component in capture-caller pause-publication pause-stop pause-dispatch; do
  bash scripts/python-entry.sh tools/proof_batch.py step "$component" -- \
    bash "scripts/verify-$component.sh"
done
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-pause-receipt.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_pause_receipt_components.py formal --output "$EVIDENCE/reproduction"
