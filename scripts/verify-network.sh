#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/network-inputs.sha256
bash scripts/python-entry.sh tools/check_network_extraction.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
cargo fmt --manifest-path verification/extract-network/Cargo.toml -- --check
cargo clippy --manifest-path verification/extract-network/Cargo.toml --locked --offline -- -D warnings
FILES=(verification/aeneas/network/SandproofNetwork/Types.lean
  verification/aeneas/network/SandproofNetwork/Funs.lean
  verification/aeneas/network/translation.json)
BEFORE="$(mktemp)"
AFTER="$(mktemp)"
cleanup() { rm -f "$BEFORE" "$AFTER"; }
trap cleanup EXIT
shasum -a 256 "${FILES[@]}" > "$BEFORE"
bash scripts/python-entry.sh tools/check_network_extraction.py --llbc-digest-only >> "$BEFORE"
bash scripts/regenerate-network-lean.sh
shasum -a 256 "${FILES[@]}" > "$AFTER"
bash scripts/python-entry.sh tools/check_network_extraction.py --llbc-digest-only >> "$AFTER"
if ! cmp -s "$BEFORE" "$AFTER"; then
  echo 'IPv4 generated semantics were stale; regenerate and recheck source-bound proofs.' >&2
  diff -u "$BEFORE" "$AFTER" >&2 || true
  exit 1
fi
bash scripts/python-entry.sh tools/check_network_extraction.py
lake build SandproofNetwork SandboxModel.Network.Audit SandboxModel.Access.Audit SandboxModel.Local.Audit
lake env lean SandboxModel/Network/Audit.lean
lake env lean SandboxModel/Access/Audit.lean
lake env lean SandboxModel/Local/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel/Network.lean SandboxModel/Network SandboxModel/ProofAuditCommand.lean \
    SandboxModel/Access SandboxModel/Local.lean SandboxModel/Local \
    verification/aeneas/network; then
  echo 'Forbidden proof placeholder or unapproved assumption in IPv4 sources.' >&2
  exit 1
fi
echo 'Listed shared-state, IPv4, network-admission and local-component proofs passed extraction consistency, compilation and dependency audit.'
echo 'Standard-library correspondence remains conditional on A6-network-stdlib.'
echo 'This is a module check, not the complete controller verification gate.'
