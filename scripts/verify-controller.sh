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
bash scripts/verify-task-contract.sh
bash scripts/verify-data-drain.sh
