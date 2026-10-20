#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/bounded-writer-inputs.sha256
bash scripts/python-entry.sh tools/check_bounded_writer.py
bash scripts/python-entry.sh tools/proof_native_suite.py require bounded-writer
: "${SANDPROOF_WRITER_LEASE:?read-only host directory lease required}"
: "${SANDPROOF_WRITER_SOURCE_ARCHIVE:?frozen original gzip archive required}"
: "${SANDPROOF_PROOF_BWRAP:?measured read-only namespace binary required}"
WRITER_EVIDENCE="$(bash scripts/python-entry.sh -c 'import os,sys,tempfile; from pathlib import Path; sys.path.insert(0,"verification/experiments/bounded-writer"); from lease import admit; paths=admit(Path(os.environ["SANDPROOF_WRITER_LEASE"]),Path(os.environ["SANDPROOF_PROOF_BWRAP"])); print(tempfile.mkdtemp(prefix="sandproof-writer-module-",dir=paths["work"]))')"
bash scripts/python-entry.sh verification/experiments/bounded-writer/inside.py run \
  --lease "$SANDPROOF_WRITER_LEASE" --source-archive "$SANDPROOF_WRITER_SOURCE_ARCHIVE" \
  --output "$WRITER_EVIDENCE/complete"
echo 'Two actual Writer variants, original regeneration, same-contract rejection, complete physical diagnostics and actual audits passed.'
