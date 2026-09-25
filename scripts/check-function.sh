#!/usr/bin/env bash
# Fast functional-development checks; no KVM, cloud access, or Lean proof claims.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

python3 -m unittest discover -s tools/tests -v
python3 tools/check_docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked

echo "Functional checks passed: workflow tests, documentation, format, lint, Rust tests and doctests."
echo "This does not run KVM system tests, E2B compatibility tests, benchmarks, or proofs."
