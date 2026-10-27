#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
# Check every retained project proof, including files omitted by an import
# aggregator. V3 files requiring different generated views must first have a
# current complete module receipt and exact physical-file coverage. They are
# checked in those original views, never silently omitted from acceptance.
# The other files use the additive Linux Lake view and unchanged dependencies.
SCOPED_PROOFS="$(mktemp "${TMPDIR:-/tmp}/sandproof-retained-views.XXXXXXXX")"
bash scripts/python-entry.sh tools/proof_batch.py retained-protocol-files > "$SCOPED_PROOFS"
PROOF_TARGETS=(SandboxModel)
while IFS= read -r proof_file; do
  if grep -Fxq "$proof_file" "$SCOPED_PROOFS"; then
    continue
  fi
  proof_module="${proof_file%.lean}"
  PROOF_TARGETS+=("${proof_module//\//.}")
done < <(rg --files SandboxModel -g '*.lean' | sort)
lake -f lake-first-chain.toml --no-cache build "${PROOF_TARGETS[@]}"
lake env lean verification/ProofAudit.lean

if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel.lean SandboxModel verification/aeneas; then
  echo "forbidden Lean proof placeholder or custom axiom found" >&2
  exit 1
fi

bash scripts/python-entry.sh tools/proof_services.py
