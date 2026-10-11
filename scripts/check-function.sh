#!/usr/bin/env bash
# Fast functional-development checks; no KVM, cloud access, or Lean proof claims.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

check_mode=linux
if [[ "${1:-}" == --portable && "$#" == 1 ]]; then
  check_mode=portable
elif [[ "$#" != 0 ]]; then
  echo 'usage: bash scripts/check-function.sh [--portable]' >&2
  exit 2
fi
bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode" --preflight-only

# Standalone node contracts must not pull in the journal/C toolchain. Feature
# unification in the complete server is expected and is checked separately.
node_dependencies="$(cargo tree --locked -p sandproof-node --edges normal,build --prefix none --format '{p}')"
if printf '%s\n' "$node_dependencies" | rg -q '^(rusqlite|libsqlite3-sys) '; then
  echo 'standalone node unexpectedly depends on SQLite' >&2
  exit 1
fi
cargo clippy --locked -p sandproof-node --lib -- -D warnings
cargo test --locked -p sandproof-controller-service --no-default-features --lib

# Build the real reducer once outside the component-test timeout. Fresh CI
# machines have no Rust cache; compilation is preparation, not a window step.
# The enclosing CI job deadline remains unchanged and a build failure is fatal.
if [[ "$check_mode" == linux ]]; then
  cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer
fi
bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode"
bash scripts/python-entry.sh tools/check_docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --locked -p sandproof-controller-service --all-targets --features contention-diagnostics -- -D warnings
cargo test --locked -p sandproof-controller-service --features contention-diagnostics

if [[ "$check_mode" == linux ]]; then
  echo "Functional checks passed: workflow tests, documentation, format, lint, Rust tests and doctests."
else
  echo "Portable development checks passed; required Linux tool groups and Linux Rust modules were NOT accepted."
fi
echo "This does not run KVM system tests, E2B compatibility tests, benchmarks, or proofs."
