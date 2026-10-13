#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

while IFS= read -r theorem; do
  if ! rg -q '^[[:space:]]*``SandboxModel\.FirstChain\.'"$theorem"'(,|\])[[:space:]]*$' SandboxModel/FirstChain/Audit.lean; then
    echo "Missing first-chain dependency audit entry: $theorem" >&2
    exit 1
  fi
done < <(rg --no-filename '^theorem [A-Za-z0-9_]+' SandboxModel/FirstChain --glob '*.lean' | awk '{print $2}')

lake build SandboxModel.FirstChain.Audit
lake env lean SandboxModel/FirstChain/Audit.lean
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' SandboxModel/FirstChain; then
  echo 'Forbidden proof placeholder or custom assumption in first-chain model.' >&2
  exit 1
fi
echo 'First-chain abstract model, scenarios and dependency audit passed.'
echo 'No Rust refinement, complete first-chain coverage or liveness claim.'
