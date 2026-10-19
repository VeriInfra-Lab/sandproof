#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/json-request-inputs.sha256
bash scripts/python-entry.sh tools/check_foundations_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
bash scripts/python-entry.sh tools/check_json_request_binding.py
cargo fmt --manifest-path verification/extract-foundations/Cargo.toml -- --check
cargo test --manifest-path verification/extract-foundations/Cargo.toml --locked --offline
cargo clippy --manifest-path verification/extract-foundations/Cargo.toml --locked --offline --all-targets -- -D warnings
FILES=(verification/aeneas/foundations/SandproofFoundations/Types.lean
  verification/aeneas/foundations/SandproofFoundations/Funs.lean
  verification/aeneas/foundations/translation.json)
BEFORE="$(mktemp)"
AFTER="$(mktemp)"
cleanup() { rm -f "$BEFORE" "$AFTER"; }
trap cleanup EXIT
shasum -a 256 "${FILES[@]}" > "$BEFORE"
bash scripts/python-entry.sh tools/check_foundations_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/regenerate-foundations-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_foundations_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'Lifecycle guard/leaf generated semantics were stale; preserve differences and recheck.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_foundations_extraction.py
bash scripts/regenerate-json-request-binding.sh
bash scripts/python-entry.sh tools/check_json_request_binding.py \
  --llbc-dir artifacts/json-request-binding \
  --library-root "${CARGO_HOME:-$HOME/.cargo}/registry/src/index.crates.io-1949cf8c6b5b557f" --check-features
while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.Boundaries\.JsonRequest\.'"$theorem"'$' \
      SandboxModel/Boundaries/JsonRequest/Audit.lean; then
    echo "Missing lifecycle JSON theorem audit: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' \
  SandboxModel/Boundaries/JsonRequest --glob '*.lean' | awk '{print $2}')
lake build SandproofFoundations.Funs SandboxModel.Boundaries.JsonRequest.Audit
lake env lean SandboxModel/Boundaries/JsonRequest/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Boundaries/JsonRequest.lean SandboxModel/Boundaries/JsonRequest \
    verification/aeneas/foundations; then
  echo 'Forbidden placeholder or custom assumption in lifecycle JSON proofs.' >&2
  exit 1
fi
NEGATIVE_ROOT="$(mktemp -d artifacts/json-request-negative.XXXXXX)"
bash scripts/python-entry.sh tools/check_json_request_negative.py --output "$NEGATIVE_ROOT/evidence"
echo 'Lifecycle JSON own functions, explicit A6/B4 binding, dependency audit and same-property negative check passed.'
echo 'This does not prove the JSON library internals, async shell or receiver business.'
