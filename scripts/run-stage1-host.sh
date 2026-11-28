#!/usr/bin/env bash
set -euo pipefail
umask 077
usage() {
    echo 'Usage: run-stage1-host.sh preflight|native CHECKOUT EXACT_COMMIT; formal CHECKOUT EXACT_COMMIT NATIVE_EVIDENCE; promotion CHECKOUT EXACT_COMMIT CANDIDATE_JSON [--native|--resume-from PRIOR_DIR]' >&2
    exit 2
}
[[ $# -ge 3 && $3 =~ ^[0-9a-f]{40}$ ]] || usage
case "$1" in
    preflight|native) [[ $# == 3 ]] || usage ;;
    formal) [[ $# == 4 ]] || usage ;;
    promotion) [[ $# == 4 || ($# == 6 && ($5 == --native || $5 == --resume-from)) ]] || usage ;;
    *) usage ;;
esac
mode=$1
checkout=$(realpath -e -- "$2")
expected=$3
[[ $(id -u) -ne 0 ]]
: "${SANDPROOF_STAGE1_STORAGE_ROOT:?set the private Stage1 data-disk root}"
: "${SANDPROOF_STAGE1_NATIVE_ROOT:?set its trusted short bind-mounted path}"
: "${SANDPROOF_STAGE1_XFS_ROOT:?set the trusted private XFS mount path}"
: "${SANDPROOF_STAGE1_NATIVE_ENV:?set the host native environment path}"
storage=$(realpath -e -- "$SANDPROOF_STAGE1_STORAGE_ROOT")
native=$(realpath -e -- "$SANDPROOF_STAGE1_NATIVE_ROOT")
xfs=$(realpath -e -- "$SANDPROOF_STAGE1_XFS_ROOT")
[[ $checkout == "$storage"/runs/* && "$storage" -ef "$native" ]]
[[ $(findmnt -n --mountpoint "$xfs" -o FSTYPE) == xfs && $native != "$xfs" ]]
xfs_options=,$(findmnt -n --mountpoint "$xfs" -o OPTIONS),
if [[ $xfs_options != *,prjquota,* && $xfs_options != *,pquota,* ]]; then
    echo 'Stage1 XFS scratch requires project quotas.' >&2
    exit 2
fi
[[ -f $native/formal.lock && ! -L $native/formal.lock ]]
cd -- "$checkout"
[[ $(git rev-parse HEAD) == "$expected" && -z $(git status --porcelain) ]]

# The proof environment is sourced at its physical checkout name.
formal_workers=${SANDPROOF_STAGE1_JOBS:-64}
source "$SANDPROOF_STAGE1_NATIVE_ENV"
cd -- "$native/${checkout#"$storage"/}"
[[ $PWD -ef "$checkout" ]]
export SANDPROOF_STAGE1_NATIVE_JOBS="${SANDPROOF_STAGE1_NATIVE_JOBS:-16}"
export SANDPROOF_STAGE1_LIGHT_JOBS="${SANDPROOF_STAGE1_LIGHT_JOBS:-64}"
export SANDPROOF_STAGE1_CPUSET="$("$SANDPROOF_PYTHON" -I -c \
    'import os; print(",".join(map(str, sorted(os.sched_getaffinity(0))[:64])))')"
export SANDPROOF_ACCEPTANCE_ROLE=native TMPDIR="$xfs"
export SANDPROOF_CHECK_RUST_THREADS="${SANDPROOF_CHECK_RUST_THREADS:-32}"
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-2}"
: "${SANDPROOF_NATIVE_OBSERVER:?set the independently installed native build observer}"
clean_environment=(env -u PYTHONPATH -u PYTHONOPTIMIZE -u SANDPROOF_REDIS_TEST_TMPDIR -u SANDPROOF_REDIS_SYSTEM_TMP_ACTIVE)
physical_evidence() {
    local path
    path=$(realpath -e -- "$1") || return
    if [[ $path == "$native"/runs/* ]]; then
        path="$storage/${path#"$native"/}"
    fi
    [[ $path == "$storage"/runs/* && $path -ef $1 ]] || return 1
    printf '%s\n' "$path"
}
if [[ $mode == formal ]]; then
    evidence=$(physical_evidence "$4")
    [[ -s $evidence/complete.json ]]
    export SANDPROOF_NATIVE_EVIDENCE="$evidence" SANDPROOF_ACCEPTANCE_ROLE=formal
    export SANDPROOF_STAGE1_JOBS="$formal_workers"
elif [[ $mode == promotion ]]; then
    candidate=$(realpath -e -- "$4")
    export SANDPROOF_STAGE1_JOBS="$formal_workers"
    export SANDPROOF_PROOF_HEAVY_LOCK="$native/formal.lock" SANDPROOF_LOCK_FILE="$native/formal.lock"
    unset SANDPROOF_STAGE1_RESUME_FROM SANDPROOF_PROOF_RECOMPUTE
else
    unset SANDPROOF_STAGE1_JOBS SANDPROOF_STAGE1_RESUME_FROM SANDPROOF_PROOF_RECOMPUTE
    clean_environment+=(-u SANDPROOF_STAGE1_JOBS -u SANDPROOF_STAGE1_RESUME_FROM -u SANDPROOF_PROOF_RECOMPUTE)
fi
export CARGO_TARGET_DIR="$native/cache/native-target"
unset PYTHONPATH PYTHONOPTIMIZE

if [[ $mode == preflight ]]; then
    exec bash scripts/python-entry.sh tools/stage1_host.py preflight "$expected" "$xfs"
fi

run=$(mktemp -d "$storage/runs/$mode.XXXXXXXX")
printf 'Evidence directory: %s\n' "$storage/runs/${run##*/}"
if [[ $mode == formal ]]; then
    mkdir -m700 "$run/lean"
    export TMPDIR="$run/lean" CARGO_TARGET_DIR="$run/formal-cargo-target"
fi
# The child unit owns the lock. The command follows its result without holding
# the lock in the calling shell.
unit="sandproof-stage1-$mode-${run##*.}"
preserve=(PATH TMPDIR CARGO_HOME RUSTUP_HOME CARGO_NET_OFFLINE CARGO_INCREMENTAL
          CARGO_BUILD_JOBS CARGO_TARGET_DIR PYTHONDONTWRITEBYTECODE
          SANDPROOF_PYTHON SANDPROOF_REDIS_SERVER SANDPROOF_STAGE1_REDIS_SHA256
          SANDPROOF_ACCEPTANCE_ROLE SANDPROOF_NATIVE_OBSERVER SANDPROOF_STAGE1_NATIVE_ENV
          SANDPROOF_STAGE1_STORAGE_ROOT SANDPROOF_STAGE1_NATIVE_ROOT
          SANDPROOF_CHECK_RUST_THREADS RUST_TEST_THREADS SANDPROOF_LOCK_FILE SANDPROOF_PROOF_HEAVY_LOCK
          SANDPROOF_STAGE1_NATIVE_JOBS SANDPROOF_STAGE1_LIGHT_JOBS SANDPROOF_STAGE1_JOBS
          SANDPROOF_STAGE1_CPUSET
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
command=(bash scripts/python-entry.sh tools/stage1_host.py "$mode" "$expected" "$xfs" "$run" "$native/formal.lock")
if [[ $mode == promotion ]]; then
    command=(bash scripts/python-entry.sh tools/stage1_host.py promotion
             --checkout "$PWD" --candidate "$candidate" --output "$run")
    if [[ $# == 6 ]]; then
        prior=$(physical_evidence "$6")
        command+=("$5" "$prior")
    fi
fi
exec sudo -n -- /usr/bin/systemd-run --wait --pipe --collect --quiet --unit="$unit" \
    --uid="$(id -u)" --gid="$(id -g)" --property="AllowedCPUs=$SANDPROOF_STAGE1_CPUSET" \
    --property=MemoryAccounting=yes --property=MemoryHigh=128G \
    --property=MemoryMax=160G --property=MemorySwapMax=0 \
    --property=OOMPolicy=continue --property=KillMode=control-group \
    --working-directory="$PWD" "${env_options[@]}" \
    "${clean_environment[@]}" "${command[@]}"
