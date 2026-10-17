#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/data-drain-inputs.sha256
bash scripts/python-entry.sh tools/check_data_drain_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
rustfmt --check --edition 2024 verification/extract-data-drain/src/lib.rs
FILES=(verification/aeneas/data-drain/SandproofDataDrain/Types.lean
  verification/aeneas/data-drain/SandproofDataDrain/Funs.lean
  verification/aeneas/data-drain/translation.json)
BEFORE="$(mktemp)"
AFTER="$(mktemp)"
cleanup() { rm -f "$BEFORE" "$AFTER"; }
trap cleanup EXIT
shasum -a 256 "${FILES[@]}" > "$BEFORE"
bash scripts/python-entry.sh tools/check_data_drain_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/regenerate-data-drain-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_data_drain_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'DataLease generated semantics were stale; preserve differences for review.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_data_drain_extraction.py
while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.Boundaries\.DataDrain\.'"$theorem"'$' \
      SandboxModel/Boundaries/DataDrain/Audit.lean; then
    echo "Missing DataLease theorem audit: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' \
  SandboxModel/Boundaries/DataDrain --glob '*.lean' | awk '{print $2}')
lake build SandproofDataDrain.Funs SandboxModel.Boundaries.DataDrain.Audit
lake env lean SandboxModel/Boundaries/DataDrain/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Boundaries/DataDrain.lean SandboxModel/Boundaries/DataDrain \
    verification/aeneas/data-drain; then
  echo 'Forbidden placeholder or custom assumption in DataLease proofs.' >&2
  exit 1
fi
echo 'DataLease call-local read policy passed source-bound extraction and theorem audits.'
echo 'A6-data-drain-read remains conditional; this does not prove physical exit or permission to release.'
