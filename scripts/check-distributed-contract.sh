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
cargo clippy --locked -p sandproof-host-service --all-targets -- -D warnings
cargo test --locked -p sandproof-host-service --all-targets
cargo test --locked -p sandproof-center-core --all-targets
cargo test --locked -p sandproof-controller-service internal_inbox
cargo clippy --locked -p sandproof-center-store --all-targets --all-features -- -D warnings
cargo test --locked -p sandproof-center-store --features redis-tests --all-targets
cargo clippy --locked -p sandproof-center-service --all-targets --all-features -- -D warnings
(
  if [[ "${GITHUB_ACTIONS:-}" == true ]]; then
    # The SDK runner's PrivateTmp isolates this directory. Center Redis tests
    # require real fsync, while save-bundle tests elsewhere require loop XFS.
    TMPDIR="$(mktemp -d /tmp/sandproof-center-redis.XXXXXXXX)"
    export TMPDIR
    trap '
      test_status=$?
      if ! rmdir -- "$TMPDIR" && (( test_status == 0 )); then
        test_status=1
      fi
      exit "$test_status"
    ' EXIT
    if [[ "$(findmnt -n -o FSTYPE --target "$TMPDIR")" != ext4 ]]; then
      echo 'Expected guest ext4 for the center Redis test fixtures.' >&2
      exit 2
    fi
  fi
  cargo test --locked -p sandproof-center-service --features redis-tests --all-targets
)
cargo clippy --locked -p sandproof-center --all-targets -- -D warnings
cargo test --locked -p sandproof-center --all-targets
cargo clippy --locked -p sandproof-server --all-targets -- -D warnings
cargo test --locked -p sandproof-server --all-targets
echo "Internal identity/lifecycle HTTPS, center business/routing decisions, Redis dispatch boundary, original SQLite atomic local admission/termination and fact projection component tests passed."
echo "Unique service parsers, center endpoint roles, real Redis/artifact HTTPS and original host application assembly passed as components; production VM startup, real guest routing and KVM were NOT tested."
