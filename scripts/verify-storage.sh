#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/storage-inputs.sha256
shasum -a 256 -c verification/foundations-inputs.sha256
: "${SANDPROOF_REDIS_SERVER:?Set the fixed real Redis binary; component tests must not be skipped}"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
mkdir -p artifacts
BATCH="$(mktemp -d "$ROOT_DIR/artifacts/storage-proof.XXXXXXXX")"
echo "Redis proof evidence: $BATCH"
# Preserve private sources and all failures for diagnosis. Never reuse an old
# destination or remove a shared .lake/build/cache to manufacture consistency.
bash scripts/python-entry.sh verification/experiments/redis-result/replay.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/correct" --tool-root "$TOOL_ROOT"
bash scripts/python-entry.sh tools/check_storage_extraction.py \
  --llbc "$BATCH/correct/source/verification/aeneas/llbc/sandproof_storage.llbc" \
  --tool-root "$TOOL_ROOT" --check-mutations
(
  cd "$BATCH/correct/source"
  echo 'Checking the actual isolated Rust candidate with strict Clippy and real Redis tests.'
  cargo clippy -p sandproof-center-store --all-targets --features redis-tests --locked --offline \
    -- -D warnings > "$BATCH/candidate-clippy.log" 2>&1
  RUST_TEST_THREADS=1 cargo test -p sandproof-center-store --features redis-tests --locked --offline \
    > "$BATCH/candidate-tests.log" 2>&1
)
# This is an additive view; the original main and first-chain views are intact.
lake -f lake-storage.toml --no-cache build SandproofStorage.Audit
lake -f lake-storage.toml env lean verification/aeneas/storage/SandproofStorage/Audit.lean
bash scripts/python-entry.sh verification/experiments/redis-result/negative.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/mutant" \
  --correct-replay "$BATCH/correct" --tool-root "$TOOL_ROOT"
bash scripts/python-entry.sh - "$BATCH" <<'PY'
from pathlib import Path
import hashlib
import json
import sys
root = Path.cwd()
batch = Path(sys.argv[1])
profile = json.loads((root / 'verification/experiments/redis-result/candidate.json').read_text())
for name, expected in profile['candidate_files'].items():
    path = batch / 'correct/source' / name
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise ValueError('correct candidate source changed during proof/test checks: ' + name)
name = 'verification/aeneas/storage/SandproofStorage/Audit.lean'
requests = [dict(tool='lean_run_code', code_file=name,
                 expected_sha256=hashlib.sha256((root / name).read_bytes()).hexdigest()), dict(tool='exit')]
(batch / 'service-requests.jsonl').write_text(''.join(json.dumps(x) + '\n' for x in requests))
PY
# The same real lean-lsp-mcp client required by the project workflow.
.tools/lean-mcp/bin/python tools/lean_session.py \
  < "$BATCH/service-requests.jsonl" > "$BATCH/service.log" 2>&1
bash scripts/python-entry.sh tools/check_storage_extraction.py \
  --llbc "$BATCH/correct/source/verification/aeneas/llbc/sandproof_storage.llbc" \
  --service-log "$BATCH/service.log"
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    verification/aeneas/storage verification/experiments/redis-result/Counterexample.lean; then
  echo 'Forbidden placeholder or custom assumption in Redis proofs.' >&2
  exit 1
fi
echo 'Redis isolated candidate passed original-tool replay, source/Drop admission, real component tests, theorem audits and actual Rust negative check.'
echo 'This is conditional on the registered library/adapter domain; concrete codec/Lua and current production/deployed-system closure remain separate.'
