#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/foundations-inputs.sha256
bash scripts/python-entry.sh tools/check_foundations_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
bash scripts/acceptance-native.sh cargo fmt --manifest-path verification/extract-foundations/Cargo.toml -- --check
bash scripts/acceptance-native.sh cargo clippy --manifest-path verification/extract-foundations/Cargo.toml --locked --offline -- -D warnings
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
  echo 'V0 generated semantics were stale; regenerate and recheck source-bound proofs.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_foundations_extraction.py
lake build SandproofFoundations SandboxModel.Foundations.Audit
lake env lean SandboxModel/Foundations/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Foundations.lean SandboxModel/Foundations SandboxModel/ProofAuditCommand.lean \
    verification/aeneas/foundations; then
  echo 'Forbidden proof placeholder or unapproved assumption in V0 sources.' >&2
  exit 1
fi
echo 'V0 listed foundations passed extraction consistency, compilation and dependency audit.'
echo 'This is a module check, not the complete controller verification gate.'
