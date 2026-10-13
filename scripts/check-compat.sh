#!/usr/bin/env bash
# Pinned SDK wire/gateway fixtures and guest codec with real commands; no VM/cloud.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

uv run --locked --project tests/compat \
  python -m unittest discover -s tests/compat -p 'test_*.py' -v

echo "SDK wire checks passed against synthetic loopback responses."

if [[ "$(uname -s)" == Linux ]]; then
  cargo clippy -p sandproof-host-service --all-targets --features sdk-contract-tests --locked -- -D warnings
  cargo test -p sandproof-host-service --features sdk-contract-tests --locked \
    data_gateway::sdk_tests:: -- --test-threads=1 --nocapture
  echo "SDK/gateway composition checks passed; authorization, host relay and VM are not tested."
  cargo clippy -p sandproof-guest-runtime --all-targets --features sdk-contract-tests --locked -- -D warnings
  cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_protocol -- --nocapture
  echo "SDK/guest codec with real commands passed through a test router; this is not a complete guest service or Go envd/KVM comparison."
else
  echo "SDK/gateway composition checks not run: the gateway requires Linux."
fi
