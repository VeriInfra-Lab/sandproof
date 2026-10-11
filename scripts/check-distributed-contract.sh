#!/usr/bin/env bash
# Targeted center/host components. No production credentials, Redis or KVM.
# Expand this same entry when the Redis adapter lands; never substitute a mock
# for the mandatory pinned Redis storage fault acceptance.
set -euo pipefail
umask 022
PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

cargo test --locked -p sandproof-protocol --test internal_contract
cargo test --locked -p sandproof-internal --all-targets
echo "Internal identity and real mutual TLS probe component tests passed."
echo "Redis coordination, durable operations, public SDK routing and KVM were NOT tested."
