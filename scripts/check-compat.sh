#!/usr/bin/env bash
# Pinned SDK wire tests and the real gateway against synthetic envd; no VM/cloud.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

uv run --locked --project tests/compat \
  python -m unittest discover -s tests/compat -p 'test_*.py' -v

echo "SDK wire checks passed against synthetic loopback responses."

if [[ "$(uname -s)" == Linux ]]; then
  cargo clippy -p sandproof-server --all-targets --features sdk-contract-tests --locked -- -D warnings
  cargo test -p sandproof-server --features sdk-contract-tests --locked \
    gateway::sdk_tests::pinned_sdk_through_gateway -- --exact --nocapture
  echo "SDK/gateway composition checks passed; authorization, host relay and VM are not tested."
else
  echo "SDK/gateway composition checks not run: the gateway requires Linux."
fi
