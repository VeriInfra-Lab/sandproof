#!/usr/bin/env bash
# Fast functional-development checks; no KVM, cloud access, or Lean proof claims.
set -euo pipefail

# Compiled checked helpers must not be group/other-writable even when a
# developer's login umask is 002. Existing cache identities are still checked.
umask 022

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

check_mode=linux
if [[ "${1:-}" == --portable && "$#" == 1 ]]; then
  check_mode=portable
elif [[ "$#" != 0 ]]; then
  echo 'usage: bash scripts/check-function.sh [--portable]' >&2
  exit 2
fi

# Diagnostics never wrap child I/O, change test options, retry commands or own
# cleanup. Keep the original shell/stdio and exit status. Receipts are private,
# bounded and fresh per invocation; a killed run remains explicitly incomplete.
check_python="$(bash scripts/python-entry.sh -c 'import sys; print(sys.executable)')"
if [[ -z "${SANDPROOF_CHECK_EVENTS:-}" ]]; then
  check_report_directory="$(mktemp -d "${TMPDIR:-/tmp}/sandproof-check.XXXXXXXX")"
  SANDPROOF_CHECK_EVENTS="$check_report_directory/events.jsonl"
fi
# A nested checker/test must never reuse the outer run's receipt. Helpers get
# the path explicitly, and the next Actions step has its own workflow env.
export -n SANDPROOF_CHECK_EVENTS
"$check_python" tools/check_stages.py init --events "$SANDPROOF_CHECK_EVENTS" --mode "$check_mode"
check_active_stage=''
check_begin() {
  check_active_stage="$1"
  "$check_python" tools/check_stages.py start --events "$SANDPROOF_CHECK_EVENTS" --stage "$1" --kind "$2"
}
check_end() {
  "$check_python" tools/check_stages.py finish --events "$SANDPROOF_CHECK_EVENTS" --stage "$check_active_stage" --exit-code 0
  check_active_stage=''
}
check_exit() {
  check_status="$?"
  # A command substitution is not the owner of this run's final receipt.
  if [[ "$BASH_SUBSHELL" != 0 ]]; then return; fi
  trap - EXIT
  if [[ -n "$check_active_stage" ]]; then
    if [[ "$check_status" == 0 ]]; then check_status=70; fi
    "$check_python" tools/check_stages.py finish --events "$SANDPROOF_CHECK_EVENTS" \
      --stage "$check_active_stage" --exit-code "$check_status" || true
  fi
  "$check_python" tools/check_stages.py end --events "$SANDPROOF_CHECK_EVENTS" --exit-code "$check_status" || {
    if [[ "$check_status" == 0 ]]; then check_status=70; fi
  }
  "$check_python" tools/check_stages.py summary --events "$SANDPROOF_CHECK_EVENTS" || {
    if [[ "$check_status" == 0 ]]; then check_status=70; fi
  }
  exit "$check_status"
}
trap check_exit EXIT

check_begin preflight setup
bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode" --preflight-only
check_end

# Long-running state-machine tests are CPU-heavy even with no other CI job.
# Optimize executable code, never remove checks. Use the same settings for dev
# helpers and tests so Cargo can reuse dependencies between original commands.
# Explicitly override ambient profile knobs: debug-only fault injection and
# arithmetic panic checks must remain enabled, including in dependencies.
export CARGO_PROFILE_DEV_OPT_LEVEL=1
export CARGO_PROFILE_TEST_OPT_LEVEL=1
export CARGO_PROFILE_DEV_DEBUG_ASSERTIONS=true
export CARGO_PROFILE_TEST_DEBUG_ASSERTIONS=true
export CARGO_PROFILE_DEV_OVERFLOW_CHECKS=true
export CARGO_PROFILE_TEST_OVERFLOW_CHECKS=true

# Standalone node contracts must not pull in the journal/C toolchain. Feature
# unification in the complete server is expected and is checked separately.
check_begin node-dependency-contract contract
node_dependencies="$(cargo tree --locked -p sandproof-node --edges normal,build --prefix none --format '{p}')"
case "$node_dependencies" in
  *'rusqlite '*|*'libsqlite3-sys '*)
    echo 'standalone node unexpectedly depends on SQLite' >&2
    exit 1
    ;;
esac
check_end
check_begin node-clippy lint
cargo clippy --locked -p sandproof-node --lib -- -D warnings
check_end
check_begin controller-no-default-tests test
cargo test --locked -p sandproof-controller-service --no-default-features --lib
check_end

# Build the real reducer once outside the component-test timeout. Fresh CI
# machines have no Rust cache; compilation is preparation, not a window step.
# The enclosing CI job deadline remains unchanged and a build failure is fatal.
if [[ "$check_mode" == linux ]]; then
  check_begin reducer-build build
  cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer
  check_end
fi
check_begin tool-tests test
bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode"
check_end
check_begin documentation documentation
bash scripts/python-entry.sh tools/check_docs.py
check_end
check_begin rust-format lint
cargo fmt --all -- --check
check_end
check_begin workspace-clippy lint
cargo clippy --workspace --all-targets --locked -- -D warnings
check_end
check_begin workspace-tests test
cargo test --workspace --all-targets --locked
check_end
check_begin workspace-doctests test
cargo test --workspace --doc --locked
check_end
check_begin diagnostics-clippy lint
cargo clippy --locked -p sandproof-controller-service --all-targets --features contention-diagnostics -- -D warnings
check_end
check_begin diagnostics-tests test
cargo test --locked -p sandproof-controller-service --features contention-diagnostics
check_end

if [[ "$check_mode" == linux ]]; then
  echo "Functional checks passed: workflow tests, documentation, format, lint, Rust tests and doctests."
else
  echo "Portable development checks passed; required Linux tool groups and Linux Rust modules were NOT accepted."
fi
echo "This does not run KVM system tests, E2B compatibility tests, benchmarks, or proofs."
