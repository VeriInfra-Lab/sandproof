#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/pause-dispatch-inputs.sha256
bash scripts/python-entry.sh tools/proof_native_suite.py require pause-dispatch --scope pause-dispatch
# The outer formal executor owns the heavy lock and provides the existing
# batch/runtime/cache context. Reuse its module receipts, without invoking an
# operator wrapper or reacquiring the same heavy lock in these child steps.
: "${SANDPROOF_FORMAL_RUNTIME:?original formal batch runtime is required}"
: "${SANDPROOF_FORMAL_CACHE:?original formal module cache is required}"
for component in capture-caller pause-publication pause-stop; do
  bash scripts/python-entry.sh tools/proof_batch.py step "$component" -- \
    bash "scripts/verify-$component.sh"
done
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-pause-dispatch.XXXXXXXX")"
bash scripts/python-entry.sh tools/check_pause_dispatch_components.py formal --output "$EVIDENCE/reproduction"
