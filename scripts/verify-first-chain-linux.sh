#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/first-chain-linux-inputs.sha256
bash scripts/python-entry.sh tools/check_first_chain_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
cargo fmt --manifest-path verification/extract-first-chain/Cargo.toml -- --check
cargo clippy --manifest-path verification/extract-first-chain/Cargo.toml --locked --offline -- -D warnings
FILES=(verification/aeneas/first-chain/SandproofFirstChain/Types.lean
  verification/aeneas/first-chain/SandproofFirstChain/Funs.lean
  verification/aeneas/first-chain/translation.json)
BEFORE="$(mktemp)"
AFTER="$(mktemp)"
cleanup() { rm -f "$BEFORE" "$AFTER"; }
trap cleanup EXIT
shasum -a 256 "${FILES[@]}" > "$BEFORE"
bash scripts/python-entry.sh tools/check_first_chain_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/regenerate-first-chain-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_first_chain_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'Linux first-chain semantics were stale; generated artifacts changed.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_first_chain_extraction.py
while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.FirstChainLinux\.'"$theorem"'$' SandboxModel/FirstChainLinux/Audit.lean; then
    echo "Missing Linux first-chain theorem audit: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' SandboxModel/FirstChainLinux --glob '*.lean' | awk '{print $2}')
lake -f lake-first-chain.toml --no-cache build SandproofFirstChain.Funs SandboxModel.FirstChainLinux.Audit
lake -f lake-first-chain.toml env lean SandboxModel/FirstChainLinux/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/FirstChainLinux verification/aeneas/first-chain; then
  echo 'Forbidden proof placeholder or custom assumption in Linux first-chain batch.' >&2
  exit 1
fi
echo 'Linux first-chain source-bound extraction and local proofs passed.'
echo 'This is not full controller/system verification; the retained complete proof gate remains mandatory.'
