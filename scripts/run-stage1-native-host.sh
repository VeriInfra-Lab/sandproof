#!/usr/bin/env bash
set -euo pipefail
umask 077
if [[ $# != 3 || ($1 != preflight && $1 != run) || ! $3 =~ ^[0-9a-f]{40}$ ]]; then
    echo 'Usage: run-stage1-native-host.sh preflight|run CHECKOUT EXACT_COMMIT' >&2
    exit 2
fi
mode=$1
checkout=$(realpath -e -- "$2")
expected=$3
: "${SANDPROOF_STAGE1_STORAGE_ROOT:?set the private Stage1 data-disk root}"
: "${SANDPROOF_STAGE1_NATIVE_ROOT:?set its trusted short bind-mounted path}"
: "${SANDPROOF_STAGE1_XFS_ROOT:?set the trusted private XFS mount path}"
: "${SANDPROOF_STAGE1_NATIVE_ENV:?set the host native environment path}"
storage=$(realpath -e -- "$SANDPROOF_STAGE1_STORAGE_ROOT")
native=$(realpath -e -- "$SANDPROOF_STAGE1_NATIVE_ROOT")
xfs=$(realpath -e -- "$SANDPROOF_STAGE1_XFS_ROOT")
[[ $checkout == "$storage"/runs/* && "$storage" -ef "$native" ]]
[[ $(findmnt -n --mountpoint "$xfs" -o FSTYPE) == xfs && $native != "$xfs" ]]
[[ -f $native/formal.lock && ! -L $native/formal.lock ]]
cd -- "$checkout"
[[ $(git rev-parse HEAD) == "$expected" && -z $(git status --porcelain) ]]

# The proof environment is sourced at its physical checkout name.
source "$SANDPROOF_STAGE1_NATIVE_ENV"
cd -- "$native/${checkout#"$storage"/}"
[[ $PWD -ef "$checkout" ]]
export SANDPROOF_STAGE1_NATIVE_JOBS="${SANDPROOF_STAGE1_NATIVE_JOBS:-16}"
export SANDPROOF_STAGE1_LIGHT_JOBS="${SANDPROOF_STAGE1_LIGHT_JOBS:-32}"
export SANDPROOF_ACCEPTANCE_ROLE=native TMPDIR="$xfs"
export CARGO_TARGET_DIR="$native/cache/native-target"
unset PYTHONPATH

if [[ $mode == preflight ]]; then
    exec bash scripts/python-entry.sh tools/stage1_native_host.py preflight "$expected" "$xfs"
fi

run=$(mktemp -d "$native/runs/native.XXXXXXXX")
printf 'Evidence directory: %s\n' "$storage/runs/${run##*/}"
mkdir -m700 "$run/redis"
export SANDPROOF_REDIS_TEST_TMPDIR="$run/redis"
# Keep the launcher lock separate from the child unit. Only the child opens and
# owns the shared lock. Losing the SSH caller cannot release it mid-run.
unit="sandproof-stage1-native-${run##*.}"
preserve=(PATH TMPDIR CARGO_HOME RUSTUP_HOME CARGO_NET_OFFLINE CARGO_INCREMENTAL
          CARGO_BUILD_JOBS CARGO_TARGET_DIR PYTHONDONTWRITEBYTECODE
          SANDPROOF_PYTHON SANDPROOF_REDIS_SERVER SANDPROOF_STAGE1_REDIS_SHA256
          SANDPROOF_REDIS_TEST_TMPDIR SANDPROOF_ACCEPTANCE_ROLE
          SANDPROOF_STAGE1_NATIVE_JOBS SANDPROOF_STAGE1_LIGHT_JOBS
          SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE
          SANDPROOF_WRITER_SOURCE_ARCHIVE SANDPROOF_LIBRARY_SOURCE_ARCHIVE
          SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE SANDPROOF_SOURCE_ROWS_PROVIDER_ROOT
          SANDPROOF_PRODUCTION_JSON_ORIGINAL_EVIDENCE XDG_CACHE_HOME UV_CACHE_DIR)
env_options=()
for name in "${preserve[@]}"; do
    if [[ -n ${!name:-} ]]; then
        env_options+=("--setenv=$name=${!name}")
    fi
done
exec systemd-run --user --wait --pipe --collect --quiet --unit="$unit" \
    --property=MemoryAccounting=yes --property=MemoryHigh=128G \
    --property=MemoryMax=160G --property=MemorySwapMax=0 \
    --property=KillMode=control-group --working-directory="$PWD" "${env_options[@]}" \
    bash scripts/python-entry.sh tools/stage1_native_host.py run "$expected" "$xfs" "$run" "$native/formal.lock"
