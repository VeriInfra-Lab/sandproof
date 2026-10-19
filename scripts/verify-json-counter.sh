#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/json-counter-inputs.sha256
bash scripts/python-entry.sh tools/check_json_counter_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
bash scripts/acceptance-native.sh cargo fmt --manifest-path verification/extract-foundations/Cargo.toml -- --check
bash scripts/acceptance-native.sh cargo test --manifest-path verification/extract-foundations/Cargo.toml --locked --offline
FILES=(verification/aeneas/foundations/SandproofFoundations/Types.lean
  verification/aeneas/foundations/SandproofFoundations/Funs.lean
  verification/aeneas/foundations/translation.json)
BEFORE="$(mktemp)"
AFTER="$(mktemp)"
cleanup() { rm -f "$BEFORE" "$AFTER"; }
trap cleanup EXIT
shasum -a 256 "${FILES[@]}" > "$BEFORE"
bash scripts/python-entry.sh tools/check_json_counter_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/proof-cache-step.sh foundations-generation bash scripts/regenerate-foundations-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_json_counter_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'JSON counter generated semantics were stale; preserve differences for review.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_json_counter_extraction.py
while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.Boundaries\.JsonCounter\.(Examples\.)?'"$theorem"'$' \
      SandboxModel/Boundaries/JsonCounter/Audit.lean; then
    echo "Missing JSON counter theorem audit: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' \
  SandboxModel/Boundaries/JsonCounter --glob '*.lean' | awk '{print $2}')
lake build SandproofFoundations.Funs SandboxModel.Boundaries.JsonCounter.Audit
lake env lean SandboxModel/Boundaries/JsonCounter/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Boundaries/JsonCounter.lean SandboxModel/Boundaries/JsonCounter \
    verification/aeneas/foundations; then
  echo 'Forbidden placeholder or custom assumption in JSON counter proofs.' >&2
  exit 1
fi
echo 'JSON counter leaf passed real-source extraction, refinement and dependency audits.'
echo 'JSON byte parsing, complete schemas and caller binding retain their explicit A6 obligations.'
