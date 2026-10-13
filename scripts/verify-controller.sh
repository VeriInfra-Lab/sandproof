#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"

GENERATED_FILES=(
  verification/aeneas/generated/SandboxControllerCore.lean
  verification/aeneas/generated/SandboxControllerCore/Types.lean
  verification/aeneas/generated/SandboxControllerCore/Funs.lean
  verification/aeneas/generated/translation.json
)
BEFORE_HASHES="$(mktemp)"
AFTER_HASHES="$(mktemp)"
cleanup() {
  rm -f "$BEFORE_HASHES" "$AFTER_HASHES"
}
trap cleanup EXIT

for generated in "${GENERATED_FILES[@]}"; do
  test -f "$generated"
done
shasum -a 256 "${GENERATED_FILES[@]}" > "$BEFORE_HASHES"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/verify-foundations.sh
bash scripts/verify-network.sh
scripts/regenerate-rust-lean.sh

shasum -a 256 "${GENERATED_FILES[@]}" > "$AFTER_HASHES"
if ! cmp -s "$BEFORE_HASHES" "$AFTER_HASHES"; then
  echo "generated Rust semantics were stale; regeneration changed checked artifacts" >&2
  diff -u "$BEFORE_HASHES" "$AFTER_HASHES" >&2 || true
  exit 1
fi

# Every explicit Rust extraction root must occur in Aeneas' machine-readable
# translation inventory.  This makes forgetting a newly declared root a hard
# verification failure instead of silently reducing proof coverage.
while IFS= read -r rust_root; do
  translated_root="${rust_root/#crate::/sandbox_controller_core::}"
  if ! rg -Fq "\"rust_name\": \"$translated_root\"" \
      verification/aeneas/generated/translation.json; then
    echo "missing Aeneas translation entry for $rust_root" >&2
    exit 1
  fi
done < <(rg -o --no-filename 'crate::[A-Za-z0-9_:]+' \
  scripts/regenerate-rust-lean.sh | sort -u)

# Check every project proof file, even if an unfinished import aggregation
# accidentally leaves it out of the default library target.  Building a
# partial import tree is not evidence that every checked-in proof compiles.
PROOF_TARGETS=(SandboxModel SandboxControllerCore)
while IFS= read -r proof_file; do
  proof_module="${proof_file%.lean}"
  PROOF_TARGETS+=("${proof_module//\//.}")
done < <(rg --files SandboxModel -g '*.lean' | sort)
lake build "${PROOF_TARGETS[@]}"
lake env lean verification/ProofAudit.lean

if rg -n --glob '*.lean' '\b(sorry|admit|axiom)\b' \
    SandboxModel.lean SandboxModel \
    verification/aeneas/generated/SandboxControllerCore.lean \
    verification/aeneas/generated/SandboxControllerCore; then
  echo "forbidden Lean proof placeholder or custom axiom found" >&2
  exit 1
fi

echo "Rust tests, Rust lint, extraction, every project proof module, theorem dependency audit, and placeholder audit passed"
echo "This checks the current proofs; it does not assert that every Rust input already has a refinement theorem."
