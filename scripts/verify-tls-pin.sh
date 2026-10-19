#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/tls-pin-inputs.sha256
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-tls-pin.XXXXXXXX")"
echo "TLS candidate evidence: $EVIDENCE/reproduction"
bash scripts/python-entry.sh tools/check_tls_pin_contract.py \
  --tool-root "${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}" \
  --output "$EVIDENCE/reproduction"
lake build SandproofTls.Funs SandboxModel.Boundaries.TlsPin.Audit
lake env lean SandboxModel/Boundaries/TlsPin/Audit.lean
echo 'TLS isolated Rust candidate, full input policy and same-property counterexample passed.'
echo 'A9/A12 and B4 remain explicit; original borrowed production and deployed TLS are not thereby verified.'

