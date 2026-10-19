#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
# Check every retained project proof, including files omitted by an import
# aggregator. The additive Linux Lake view also declares the first-chain
# extraction library; it is checked against the unchanged base dependencies.
PROOF_TARGETS=(SandboxModel)
while IFS= read -r proof_file; do
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
