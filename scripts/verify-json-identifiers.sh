#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/json-identifiers-inputs.sha256
bash scripts/python-entry.sh tools/check_foundations_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
bash scripts/python-entry.sh tools/check_json_identifiers_extraction.py \
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
bash scripts/python-entry.sh tools/check_foundations_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/proof-cache-step.sh foundations-generation bash scripts/regenerate-foundations-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_foundations_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'JSON identifier generated semantics were stale; preserve differences for review.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_json_identifiers_extraction.py
while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.Boundaries\.JsonIdentifiers\.'"$theorem"'$' \
      SandboxModel/Boundaries/JsonIdentifiers/Audit.lean; then
    echo "Missing JSON identifier theorem audit: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' \
  SandboxModel/Boundaries/JsonIdentifiers --glob '*.lean' | awk '{print $2}')
lake build SandproofFoundations.Funs SandboxModel.Boundaries.JsonIdentifiers.Audit
lake env lean SandboxModel/Boundaries/JsonIdentifiers/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Boundaries/JsonIdentifiers.lean SandboxModel/Boundaries/JsonIdentifiers \
    SandboxModel/Foundations/IdentifierModel.lean verification/aeneas/foundations; then
  echo 'Forbidden placeholder or custom assumption in JSON identifier proofs.' >&2
  exit 1
fi
echo 'Eight JSON identifier leaves passed real-source extraction, refinement and dependency audits.'
echo 'JSON parsing, complete schemas, caller binding and observable custom Drop retain explicit obligations.'
