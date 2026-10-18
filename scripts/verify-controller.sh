#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

# Reject stale child or parent manifests before starting expensive checks.
# Each module still repeats its own admission and regeneration checks below.
for proof_inputs in verification/*inputs.sha256; do
  shasum -a 256 -c "$proof_inputs"
done

# The obsolete whole-controller extraction and its RustRefinement dependants
# were retired by the project owner. Each retained source-bound module still
# regenerates and compares its complete artifacts in its own mandatory gate.
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/verify-json-identifiers.sh
bash scripts/verify-json-counter.sh
bash scripts/verify-task-contract.sh
bash scripts/verify-revocation-contract.sh
bash scripts/verify-tunnel.sh
bash scripts/verify-data-drain.sh
bash scripts/verify-foundations.sh
bash scripts/verify-network.sh
bash scripts/verify-first-chain-linux.sh
bash scripts/verify-first-chain.sh

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

echo "Rust tests, Rust lint, retained extraction consistency, every retained project proof module, theorem dependency audit, and placeholder audit passed"
echo "This checks the retained proof inventory; it does not establish current whole-controller or deployed-system refinement."
