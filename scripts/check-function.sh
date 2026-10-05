#!/usr/bin/env bash
# Fast functional-development checks; no KVM, cloud access, or Lean proof claims.
set -euo pipefail
# Keep newly compiled helpers/ancestors private even with a caller umask of
# 0002. Do not chmod pre-existing/shared build outputs to manufacture trust.
umask 077

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# This suite executes the protected ELF reducer through /proc/self/fd and
# checks Linux filesystem behaviour. Reject unsupported environments before
# compilation/tests; a portable subset is not a complete functional pass.
PYTHON="${PYTHON:-python3}"
"$PYTHON" - <<'PY'
import sys

if sys.version_info < (3, 11):
    raise SystemExit("Functional checks require Python 3.11+; select it with PYTHON=/absolute/path/to/python3.")
if sys.platform != "linux":
    raise SystemExit("Full functional checks require Linux (protected ELF helper and /proc/self/fd). Run this entry on Linux; macOS component checks do not replace the full suite.")
PY

# Build the real reducer once outside the component-test timeout. Fresh CI
# machines have no Rust cache; compilation is preparation, not a window step.
# The enclosing CI job deadline remains unchanged and a build failure is fatal.
cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer
"$PYTHON" -m unittest discover -s tools/tests -v
"$PYTHON" tools/check_docs.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked

echo "Functional checks passed: workflow tests, documentation, format, lint, Rust tests and doctests."
echo "This does not run KVM system tests, E2B compatibility tests, benchmarks, or proofs."
