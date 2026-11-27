#!/usr/bin/env bash
# Re-extract the two B views from the recorded production tree and fixed tools.
set -euo pipefail
umask 077
if [[ $# != 4 ]]; then
  echo 'usage: extract.sh NEW_OUTPUT EXISTING_HEAVY_LOCK CHARON AENEAS' >&2
  exit 2
fi

here="$(cd -- "$(dirname -- "$0")" && pwd)"
repo="$(git -C "$here" rev-parse --show-toplevel)"
out="$(realpath -m -- "$1")"
lock="$(realpath -e -- "$2")"
charon="$(realpath -e -- "$3")"
aeneas="$(realpath -e -- "$4")"
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 PYTHONDONTWRITEBYTECODE=1
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
: "${CARGO_HOME:?use the fixed offline Cargo home}"
CARGO_HOME="$(realpath -e -- "$CARGO_HOME")"
case "$out/" in "$repo/"*) echo 'use a private output outside the checkout' >&2; exit 2;; esac
[[ -f "$lock" ]]
exec 9<"$lock"
flock -x 9
mkdir -- "$out"

input() {
  python3 - "$here/inputs.json" "$@" <<'PYTHON'
import json
import sys
with open(sys.argv[1]) as source:
    value = json.load(source)
for key in sys.argv[2:]:
    value = value[key]
print(value)
PYTHON
}
base="$(input production)"
[[ "$(git -C "$repo" rev-parse "$base^{tree}")" == "$(input production_tree)" ]]
printf '%s  %s\n' "$(input tools charon_sha256)" "$charon" "$(input tools aeneas_sha256)" "$aeneas" | sha256sum -c -
sysroot="$("$charon" toolchain-path)"
export CARGO_TARGET_DIR="$out/cargo"
trap 'rm -rf -- "$CARGO_TARGET_DIR"' EXIT
entry=crate::controller::lifecycle_inbox::source_work::restore

for mode in work identity; do
  view="$out/$mode"
  mkdir -p -- "$view/source"
  git -C "$repo" archive "$base" | tar -xf - -C "$view/source"
  python3 "$here/stage.py" rust "$view/source" "$mode"
  module="$(input generated "$mode" module)"
  if [[ "$mode" == work ]]; then
    roots=("$entry::work"
      crate::proof_restore_activation_carrier::construct_permit
      crate::proof_restore_activation_carrier::permit_action
      crate::proof_restore_activation_carrier::permit_work_binding
      "$entry::proof_return_permit_after_identity")
    extra=()
  else
    roots=("$entry::proof_finalize_read_permit")
    extra=(--opaque rusqlite::types::from_sql::FromSqlError)
  fi
  args=()
  for root in "${roots[@]}"; do args+=(--start-from "$root"); done
  cd "$view/source"
  RUSTFLAGS="--remap-path-prefix=$PWD=. --remap-path-prefix=$sysroot=<extraction-sysroot> --remap-path-prefix=$CARGO_HOME=<cargo-cache>" \
    "$charon" cargo --preset=aeneas --error-on-warnings --sysroot default \
    --include sandproof_controller_service --include sandproof_protocol --include sandbox_controller_core \
    --opaque rusqlite::error::Error --opaque serde_json::error::Error \
    --opaque rusqlite::transaction --opaque rusqlite::Connection "${extra[@]}" "${args[@]}" \
    --dest-file "$view/permit-return.llbc" -- --manifest-path controller-service/Cargo.toml --lib \
    --locked --offline --target x86_64-unknown-linux-gnu >"$view/charon.log" 2>&1
  "$aeneas" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
    -namespace "${module}Rust" -subdir "$module" -dest "$view/generated" \
    "$view/permit-return.llbc" >"$view/aeneas.log" 2>&1
done
