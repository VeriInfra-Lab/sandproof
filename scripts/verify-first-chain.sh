#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

shasum -a 256 -c verification/first-chain-inputs.sha256

# Identity composition imports source-bound V0 results; recheck their actual
# Rust roots, versions and regenerated semantics before accepting this batch.
bash scripts/proof-cache-step.sh foundations bash scripts/verify-foundations.sh

while IFS= read -r theorem; do
  if ! rg -q '^#audit_controller_proof SandboxModel\.FirstChain\.'"$theorem"'$' SandboxModel/FirstChain/Audit.lean; then
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
echo 'First-chain model, extracted identity guard, conditional delivery contract and dependency audit passed.'
echo 'Full lifecycle response handling, durable commits, host evidence and liveness remain open.'
