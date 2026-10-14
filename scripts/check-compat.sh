#!/usr/bin/env bash
# Pinned SDK fixtures, real guest commands and init/MMDS components; no VM/cloud.
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
  cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_files -- --nocapture
  cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_file_io -- --nocapture
  cargo test -p sandproof-guest-runtime --features sdk-contract-tests --locked --test sdk_file_transfer -- --nocapture
  cargo test -p sandproof-guest-runtime --locked --test file_protocol
  cargo test -p sandproof-guest-runtime --locked --test filesystem
  cargo test -p sandproof-guest-runtime --locked --test file_transfer
  cargo test -p sandproof-guest-runtime --locked --lib --test context --test owner --test preparation --test pty
  echo "SDK/guest commands and real unary/raw/multipart files passed through test routers; this is not a complete guest service or Go envd/KVM comparison."
  cargo clippy -p sandproof-guest-service --all-targets --locked -- -D warnings
  cargo test -p sandproof-guest-service --all-targets --locked
  echo "Guest init/MMDS, account/context, PTY and owned command lifecycle components passed; production command routes, privileged switching and real Firecracker MMDS remain separate checks."
else
  echo "SDK/gateway composition checks not run: the gateway requires Linux."
fi
