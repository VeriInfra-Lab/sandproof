#!/usr/bin/env bash
set -euo pipefail

suite=${1##*/}
suite=${suite%-*}
threads=$SANDPROOF_CHECK_RUST_THREADS
case "$suite" in
  managed_port|business_authority|redis_https_application|redis_ledger)
    # These suites wait for real durable I/O under application deadlines.
    if [[ $threads != 1 && $threads != 2 ]]; then
      threads=2
      printf 'Rust test threads (%s): %s\n' "$suite" "$threads"
    fi
    ;;
esac
exec "$@" "--test-threads=$threads"
