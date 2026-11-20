#!/usr/bin/env bash
# Limited formal entry for the selected restore decisions.
set -euo pipefail
umask 077

if [[ $# != 5 ]]; then
  echo 'usage: verify-decisions.sh NEW_OUTPUT LOCK TOOL_BIN LAKE_PACKAGES LEAN_MCP_ENV' >&2
  exit 2
fi
here="$(cd -- "$(dirname -- "$0")" && pwd)"
repo="$(git -C "$here" rev-parse --show-toplevel)"
output="$(realpath -m -- "$1")"
lock="$(realpath -e -- "$2")"
tool_bin="$(realpath -e -- "$3")"
packages="$(realpath -e -- "$4")"
mcp_env="$(realpath -e -- "$5")"
case "$output/" in "$repo/"*) echo 'use an output outside the checkout' >&2; exit 2;; esac
[[ -f "$lock" && -d "$packages" && -x "$mcp_env/bin/lean-lsp-mcp" ]]

head="$(git -C "$repo" rev-parse HEAD)"
git -C "$repo" merge-base --is-ancestor 1b15872a07568f9f34aad514a64673ac328c7ee8 "$head"
[[ -z "$(git -C "$repo" status --porcelain)" ]]
mkdir -- "$output"
exec 9<"$lock"
flock -x 9
git -C "$repo" rev-parse HEAD 'HEAD^{tree}' > "$output/source-git"
sha256sum "$tool_bin/charon" "$tool_bin/aeneas" > "$output/tools.sha256"
printf '%s  %s\n' \
  2603f720f0d26a4c77ae215485e2457029d69504dbd9d318a497ed866f21d4fd "$tool_bin/charon" \
  22b326476f854e19b52ca5a2acfd953fd186da7694cd10d8d008ed5502f7c46a "$tool_bin/aeneas" |
  sha256sum -c -
lean --version | tee "$output/lean-version"
grep -Eq 'version 4\.31\.0,' "$output/lean-version"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
export PYTHONDONTWRITEBYTECODE=1 TERM=dumb LC_ALL=C
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
entry=crate::controller::lifecycle_inbox::source_work::restore

for view in correct wrong; do
  data="$output/$view"
  project="$data/source"
  mkdir -p -- "$project"
  git -C "$repo" archive "$head" | tar -xf - -C "$project"
  args=()
  roots=(--start-from "$entry"::proof_claim_activation_row)
  target=Readback
  if [[ "$view" == correct ]]; then
    roots+=(--start-from "$entry"::proof_existing_restore --start-from "$entry"::proof_resume_prefix)
  else
    args+=(--wrong)
    target=Wrong
  fi
  bash "$repo/scripts/python-entry.sh" "$here/stage_decisions.py" "$project" "${args[@]}"
  file=controller-service/src/controller/lifecycle_inbox/source_work/restore.rs
  sha256sum "$project/$file" > "$data/selected-source.sha256"
  cd "$project"
  export CARGO_TARGET_DIR="$output/extract-target"
  "$tool_bin/charon" cargo --preset=aeneas --error-on-warnings --sysroot default \
    --include sandproof_protocol --include sandbox_controller_core --include sandproof_controller_service \
    "${roots[@]}" --dest-file "$data/decision.llbc" -- \
    --manifest-path controller-service/Cargo.toml --lib --locked --offline --target x86_64-unknown-linux-gnu \
    > "$data/charon.log" 2>&1
  "$tool_bin/aeneas" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
    -namespace SandproofRestoreClaimRust -subdir SandproofRestoreClaim \
    -dest "$data/generated" "$data/decision.llbc" > "$data/aeneas.log" 2>&1
  mkdir .tools .lake artifacts
  ln -s -- "$packages" .lake/packages
  ln -s -- "$mcp_env" .tools/lean-mcp
  bash "$repo/scripts/python-entry.sh" "$here/stage_lean.py" "$project" "$data/generated" "${args[@]}"
  lake build "SandboxModel.Lifecycle.Protocol.RestoreClaim.$target" > "$data/build.log" 2>&1
  bash "$repo/scripts/python-entry.sh" "$here/check_view.py" "$project" "$data/check" "$target"
done

sha256sum "$output"/{correct,wrong}/decision.llbc "$output"/{correct,wrong}/check/result.json \
  > "$output/decisions-complete.sha256"
