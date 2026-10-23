#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/std-mutex-inputs.sha256
bash scripts/python-entry.sh tools/check_std_mutex_contract.py
EVIDENCE="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-std-mutex.XXXXXXXX")"
cp verification/aeneas/llbc/sandproof_std_mutex.llbc "$EVIDENCE/frozen.llbc"
cp -R verification/aeneas/std-mutex "$EVIDENCE/frozen-generated"
bash scripts/regenerate-std-mutex-lean.sh
cp verification/aeneas/llbc/sandproof_std_mutex.llbc "$EVIDENCE/regenerated.llbc"
bash scripts/python-entry.sh - "$EVIDENCE" <<'PY'
from pathlib import Path
import json,sys
from tools.check_foundations_extraction import llbc_digest
out=Path(sys.argv[1]);root=Path.cwd()
frozen=out/'frozen.llbc';actual=root/'verification/aeneas/llbc/sandproof_std_mutex.llbc'
if llbc_digest(json.loads(frozen.read_text()))!=llbc_digest(json.loads(actual.read_text())):
    raise SystemExit('Full standard Mutex LLBC differs; no ordered fields may be discarded')
for name in ('SandproofStdMutex/Types.lean','SandproofStdMutex/Funs.lean','translation.json'):
    if (out/'frozen-generated'/name).read_bytes()!=(root/'verification/aeneas/std-mutex'/name).read_bytes():
        raise SystemExit('Original standard Mutex generated file differs: '+name)
# Keep the raw regeneration above. Only the reviewed unordered map may differ.
if frozen.read_bytes()!=actual.read_bytes():
    actual.write_bytes(frozen.read_bytes())
PY
bash scripts/python-entry.sh tools/check_std_mutex_contract.py
bash scripts/python-entry.sh verification/experiments/std-mutex/preflight.py --output "$EVIDENCE/shared-preflight"
lake build SandboxModel.Boundaries.StdMutex.Audit
lake env lean SandboxModel/Boundaries/StdMutex/Audit.lean
bash scripts/python-entry.sh tools/proof_native_suite.py require std-mutex
bash scripts/python-entry.sh verification/experiments/std-mutex/negative.py --output "$EVIDENCE/wrong-rust"
echo 'Standard Mutex conditional guard rules, actual get_mut and same-contract wrong Rust refutation passed.'
echo 'Shared-lock source bindings remain A6/B2 conditions; no whole business callback or shared-lock Rust refinement claim.'
