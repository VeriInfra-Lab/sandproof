#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/sqlite-inputs.sha256
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
mkdir -p artifacts
BATCH="$(mktemp -d "$ROOT_DIR/artifacts/sqlite-proof.XXXXXXXX")"
echo "SQLite proof evidence: $BATCH"
# Each source is a new private checkout. Failures, templates and raw ASTs stay
# available; no production file, toolchain or shared cache is replaced.
bash scripts/python-entry.sh verification/experiments/sqlite-commit/replay.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/correct" --tool-root "$TOOL_ROOT"
bash scripts/python-entry.sh tools/check_sqlite_extraction.py \
  --evidence "$BATCH/correct/evidence" --tool-root "$TOOL_ROOT" --check-mutations
bash scripts/python-entry.sh verification/experiments/sqlite-commit/native.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/native"
lake -f lake-sqlite.toml --no-cache build SandproofSqlite.Audit
lake -f lake-sqlite.toml env lean verification/aeneas/sqlite/SandproofSqlite/Audit.lean
bash scripts/python-entry.sh verification/experiments/sqlite-commit/negative.py \
  --source-base "$ROOT_DIR" --destination "$BATCH/mutant" \
  --correct-replay "$BATCH/correct" --tool-root "$TOOL_ROOT"
bash scripts/python-entry.sh - "$BATCH" <<'PY'
from pathlib import Path
import hashlib,json,sys
root=Path.cwd()
name="verification/aeneas/sqlite/SandproofSqlite/Audit.lean"
requests=[dict(tool="lean_diagnostic_messages",arguments={"file_path":name},
               expected_sha256=hashlib.sha256((root/name).read_bytes()).hexdigest()),dict(tool="exit")]
(Path(sys.argv[1])/"requests.jsonl").write_text("".join(json.dumps(x)+"\n" for x in requests))
PY
.tools/lean-mcp/bin/python tools/lean_session.py \
  < "$BATCH/requests.jsonl" > "$BATCH/service.log" 2>&1
bash scripts/python-entry.sh tools/check_sqlite_extraction.py \
  --evidence "$BATCH/correct/evidence" --service-log "$BATCH/service.log" --check-mutations
if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    verification/aeneas/sqlite verification/experiments/sqlite-commit/Counterexample.lean; then
  echo 'Forbidden placeholder or custom assumption in SQLite proofs.' >&2
  exit 1
fi
echo 'SQLite candidate passed original-tool replay, complete AST/Drop admission, native checks, unchanged-property rejection and actual theorem audits.'
echo 'Conditional core proof: registered adapters, original/candidate equivalence and V5 business obligations remain explicit.'
