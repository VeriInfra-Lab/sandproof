#!/usr/bin/env bash
# Fast functional-development checks; no KVM, cloud access, or Lean proof claims.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

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
cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer
python3 -m unittest discover -s tools/tests -v
python3 tools/check_docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
cargo clippy --locked -p sandproof-controller-service --all-targets --features contention-diagnostics -- -D warnings
cargo test --locked -p sandproof-controller-service --features contention-diagnostics

echo "Functional checks passed: workflow tests, documentation, format, lint, Rust tests and doctests."
echo "This does not run KVM system tests, E2B compatibility tests, benchmarks, or proofs."
