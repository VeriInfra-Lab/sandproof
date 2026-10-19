#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/tunnel-inputs.sha256
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
mkdir -p artifacts
BATCH="$(mktemp -d "$ROOT_DIR/artifacts/tunnel-proof.XXXXXXXX")"
echo "Relay proof evidence: $BATCH"
bash scripts/python-entry.sh verification/experiments/tunnel-contract/replay.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/replay" --tool-root "$TOOL_ROOT"
lake -f lake-tunnel.toml --no-cache build SandproofTunnel.Audit
lake -f lake-tunnel.toml env lean verification/aeneas/tunnel/SandproofTunnel/Audit.lean
bash scripts/python-entry.sh - "$BATCH" <<'PY'
from pathlib import Path
import hashlib,json,sys
name="verification/aeneas/tunnel/SandproofTunnel/Audit.lean"
requests=[dict(tool="lean_diagnostic_messages",arguments={"file_path":name},
               expected_sha256=hashlib.sha256(Path(name).read_bytes()).hexdigest()),dict(tool="exit")]
(Path(sys.argv[1])/"requests.jsonl").write_text("".join(json.dumps(x)+"\n" for x in requests))
PY
.tools/lean-mcp/bin/python tools/lean_session.py \
  < "$BATCH/requests.jsonl" > "$BATCH/service.log" 2>&1
bash scripts/python-entry.sh tools/check_tunnel_extraction.py \
  --llbc "$BATCH/replay/sandproof_tunnel_policy.llbc" --tool-root "$TOOL_ROOT" \
  --service-log "$BATCH/service.log" --check-mutations
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    verification/aeneas/tunnel verification/experiments/tunnel-contract/Counterexample.lean; then
  echo 'Forbidden placeholder or custom assumption in relay proofs.' >&2
  exit 1
fi
echo 'Relay candidate passed actual source/AST/Drop replay, native checks, unchanged-property rejection and real theorem audits.'
echo 'A6/B3/B4 remain explicit; raw relay completion is not protocol success or physical drain.'
