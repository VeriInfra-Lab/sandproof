#!/usr/bin/env bash
# Targeted components with mandatory pinned real Redis. No production keys/KVM.
set -euo pipefail
umask 022
PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

if [[ -z "${SANDPROOF_REDIS_SERVER:-}" || "${SANDPROOF_REDIS_SERVER}" != /* || ! -x "${SANDPROOF_REDIS_SERVER}" ]]; then
  echo 'Set SANDPROOF_REDIS_SERVER to the absolute executable of the locked Redis test build; no skip/fallback.' >&2
  exit 2
fi

cargo test --locked -p sandproof-protocol --test internal_contract
cargo test --locked -p sandproof-internal --all-targets
cargo test --locked -p sandproof-center-core --all-targets
cargo test --locked -p sandproof-controller-service internal_inbox
cargo clippy --locked -p sandproof-center-store --all-targets --all-features -- -D warnings
cargo test --locked -p sandproof-center-store --features redis-tests --all-targets
echo "Internal identity/TLS probe, pure center decisions, Redis dispatch boundary and SQLite durable inbox component tests passed."
echo "High-level lifecycle HTTPS/local acceptance/execution, result retirement, public SDK routing and KVM were NOT tested."
