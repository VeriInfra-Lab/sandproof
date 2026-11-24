#!/usr/bin/env bash
# Run in an isolated msra Lean project. Only the project build and selected cache are writable.
set -euo pipefail
umask 077
base=/mnt/data2tb/ning/sandproof-stage1
project=$(pwd -P)
cache=${SANDPROOF_DEV_LAKE_CACHE:?set a private cache directory under /mnt/data2tb/ning}
limit=${SANDPROOF_DEV_LAKE_LIMIT_MIB:?set the cache admission threshold in MiB}
[[ $# -gt 0 && "$limit" =~ ^[1-9][0-9]{0,6}$ ]]
[[ -z ${SANDPROOF_ACCEPTANCE_ROLE:-} && "$project" == /mnt/data2tb/ning/* ]]
[[ -f lakefile.toml && -f lean-toolchain && "$(cat lean-toolchain)" == leanprover/lean4:v4.31.0 ]]
[[ -d .lake/packages && "$cache" == /mnt/data2tb/ning/* ]]
for target in "$@"; do [[ "$target" =~ ^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$ ]]; done
mkdir -p -- "$cache"
cache=$(realpath "$cache")
[[ "$cache" == /mnt/data2tb/ning/* && -O "$cache" && "$(stat -c %a "$cache")" == 700 ]]
used=$(du -sm "$cache" | cut -f1)
if (( used >= limit )); then echo "Development cache at threshold: $used MiB of $limit MiB" >&2; exit 1; fi
config=$(mktemp "$cache/.package-config.XXXXXXXX")
trap 'rm -f -- "$config"' EXIT
{ echo 'enableArtifactCache = true'; cat lakefile.toml; } > "$config"
lake_bin="$base/runtime/lean-4.31.0-linux/bin"
export PATH="$lake_bin:$PATH" LAKE_CACHE_DIR="$cache" LAKE_ARTIFACT_CACHE=0 LAKE_RESTORE_ARTIFACTS=1
mkdir -p .lake/build .lake/config
# Bind only the private build and Lake configuration outputs. Source, package inputs, and the global tool installation stay read-only.
bwrap --ro-bind / / --dev /dev --proc /proc --unshare-net --tmpfs /tmp \
    --bind "$cache" "$cache" --bind "$project/.lake/build" "$project/.lake/build" \
    --bind "$project/.lake/config" "$project/.lake/config" \
    --ro-bind "$config" "$project/lakefile.toml" --chdir "$project" \
    "$lake_bin/lake" build "$@"
