#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
# Source admission must precede even tool/runtime startup and receipt lookup.
for proof_inputs in verification/*inputs.sha256; do
  shasum -a 256 -c "$proof_inputs"
done
bash scripts/verify-use-inventory.sh
: "${SANDPROOF_NATIVE_EVIDENCE:?ning native acceptance evidence is required}"
# Ordinary tests have mandatory exact-input evidence from ning. This process
# runs formal verification only; no native suite or CI environment is started.
export SANDPROOF_ACCEPTANCE_ROLE=formal
exec bash scripts/python-entry.sh tools/proof_batch.py run \
  --cache "${SANDPROOF_FORMAL_CACHE:-$ROOT_DIR/artifacts/proof-cache}" "$@"
