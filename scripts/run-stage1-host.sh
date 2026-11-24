#!/usr/bin/env bash
set -euo pipefail
umask 077
if [[ $# -lt 3 || ! $3 =~ ^[0-9a-f]{40}$ ]] ||
   [[ ($1 == formal && $# != 4) || ($1 != formal && $# != 3) ]] ||
   [[ $1 != preflight && $1 != native && $1 != formal ]]; then
    echo 'Usage: run-stage1-host.sh preflight|native CHECKOUT EXACT_COMMIT or formal CHECKOUT EXACT_COMMIT NATIVE_EVIDENCE' >&2
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
formal_workers=${SANDPROOF_STAGE1_JOBS:-8}
source "$SANDPROOF_STAGE1_NATIVE_ENV"
cd -- "$native/${checkout#"$storage"/}"
[[ $PWD -ef "$checkout" ]]
export SANDPROOF_STAGE1_NATIVE_JOBS="${SANDPROOF_STAGE1_NATIVE_JOBS:-16}"
export SANDPROOF_STAGE1_LIGHT_JOBS="${SANDPROOF_STAGE1_LIGHT_JOBS:-32}"
export SANDPROOF_ACCEPTANCE_ROLE=native TMPDIR="$xfs"
clean_environment=(env -u PYTHONPATH)
if [[ $mode == formal ]]; then
    evidence=$(realpath -e -- "$4")
    [[ $evidence == "$storage"/runs/* || $evidence == "$native"/runs/* ]]
    [[ -s $evidence/complete.json ]]
    export SANDPROOF_NATIVE_EVIDENCE="$evidence" SANDPROOF_ACCEPTANCE_ROLE=formal
    export SANDPROOF_STAGE1_JOBS="$formal_workers"
else
    unset SANDPROOF_STAGE1_JOBS SANDPROOF_STAGE1_RESUME_FROM SANDPROOF_PROOF_RECOMPUTE
    clean_environment+=(-u SANDPROOF_STAGE1_JOBS -u SANDPROOF_STAGE1_RESUME_FROM -u SANDPROOF_PROOF_RECOMPUTE)
fi
export CARGO_TARGET_DIR="$native/cache/native-target"
unset PYTHONPATH

if [[ $mode == preflight ]]; then
    exec bash scripts/python-entry.sh tools/stage1_host.py preflight "$expected" "$xfs"
fi

run=$(mktemp -d "$native/runs/$mode.XXXXXXXX")
printf 'Evidence directory: %s\n' "$storage/runs/${run##*/}"
if [[ $mode == native ]]; then
    mkdir -m700 "$run/redis"
    export SANDPROOF_REDIS_TEST_TMPDIR="$run/redis"
else
    mkdir -m700 "$run/lean"
    export TMPDIR="$run/lean" CARGO_TARGET_DIR="$run/target"
fi
# The child unit owns the lock. The command follows its result without holding
# the lock in the calling shell.
unit="sandproof-stage1-$mode-${run##*.}"
preserve=(PATH TMPDIR CARGO_HOME RUSTUP_HOME CARGO_NET_OFFLINE CARGO_INCREMENTAL
          CARGO_BUILD_JOBS CARGO_TARGET_DIR PYTHONDONTWRITEBYTECODE
          SANDPROOF_PYTHON SANDPROOF_REDIS_SERVER SANDPROOF_STAGE1_REDIS_SHA256
          SANDPROOF_REDIS_TEST_TMPDIR SANDPROOF_ACCEPTANCE_ROLE
          SANDPROOF_STAGE1_NATIVE_JOBS SANDPROOF_STAGE1_LIGHT_JOBS SANDPROOF_STAGE1_JOBS
          SANDPROOF_NATIVE_EVIDENCE SANDPROOF_FORMAL_CACHE SANDPROOF_STAGE1_RESUME_FROM SANDPROOF_PROOF_RECOMPUTE
          SANDPROOF_PROOF_RUNTIME_ROOT SANDPROOF_PROOF_TOOL_ROOT SANDPROOF_FORMAL_PACKAGES
          SANDPROOF_STAGE1_SOURCE_SAVE_RESULT ELAN_HOME ELAN_TOOLCHAIN LEAN_NUM_THREADS LEAN_STACK_SIZE_KB
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
    "${clean_environment[@]}" bash scripts/python-entry.sh tools/stage1_host.py \
    "$mode" "$expected" "$xfs" "$run" "$native/formal.lock"
