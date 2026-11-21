#!/usr/bin/env bash
# Build and inspect the exact read-side files using the project's registered Lean service.
set -euo pipefail
umask 077
if [[ $# != 8 ]]; then
  echo 'usage: check.sh NEW_OUTPUT EXISTING_HEAVY_LOCK LAKE_PACKAGES LEAN_MCP_ENV BWRAP QUERY_GENERATED WORK_GENERATED IDENTITY_GENERATED' >&2
  exit 2
fi

here="$(cd -- "$(dirname -- "$0")" && pwd)"
repo="$(git -C "$here" rev-parse --show-toplevel)"
out="$(realpath -m -- "$1")"
lock="$(realpath -e -- "$2")"
packages="$(realpath -e -- "$3")"
mcp="$(realpath -e -- "$4")"
bwrap="$(realpath -e -- "$5")"
query="$(realpath -e -- "$6")"
work="$(realpath -e -- "$7")"
identity="$(realpath -e -- "$8")"
case "$out/" in "$repo/"*) echo 'use a private output outside the checkout' >&2; exit 2;; esac
[[ -f "$lock" && -x "$mcp/bin/lean-lsp-mcp" && -x "$bwrap" ]]
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
export LEAN_NUM_THREADS=1 PYTHONDONTWRITEBYTECODE=1
"$mcp/bin/python" --version >"$out/client-python-version"
lean --version >"$out/lean-version"
[[ "$(cat "$out/lean-version")" == "$(input tools lean_version)" ]]
[[ "$(git -C "$packages/aeneas" rev-parse HEAD)" == "$(input tools aeneas_commit)" ]]

prepare() {
  local view=$1 commit=$2 tree=$3
  [[ "$(git -C "$repo" rev-parse "$commit^{tree}")" == "$tree" ]]
  mkdir -- "$view"
  git -C "$repo" archive "$commit" | tar -xf - -C "$view"
  mkdir -- "$view/.lake" "$view/.tools"
  ln -s -- "$packages" "$view/.lake/packages"
  ln -s -- "$mcp" "$view/.tools/lean-mcp"
}

selection="$out/selection"
permit="$out/permit"
prepare "$selection" "$(input query_provider)" "$(input query_provider_tree)"
python3 "$here/stage.py" query "$selection" "$query"
prepare "$permit" "$(input production)" "$(input production_tree)"
python3 "$here/stage.py" lean "$permit" "$work" "$identity"

(cd "$selection" && lake build PermitReadSelection) >"$out/selection-build.log" 2>&1
(cd "$permit" && lake build PermitReadWork PermitReadReturn PermitReadIdentity) >"$out/permit-build.log" 2>&1

export TMPDIR="$out/service-tmp" ELAN_HOME="$out/elan-empty"
mkdir -- "$TMPDIR"
diagnose() {
  local view=$1 report=$2
  shift 2
  python3 "$here/audit.py" requests "$view" "$@" >"$report.requests.jsonl"
  "$bwrap" --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --bind "$out" "$out" \
    --chdir "$view" "$mcp/bin/python" tools/lean_session.py \
    <"$report.requests.jsonl" >"$report.jsonl" 2>"$report.stderr"
  python3 "$here/audit.py" results "$view" "$report.jsonl" "$@"
}
diagnose "$selection" "$out/selection-service" PermitReadSelection.lean \
  SandboxModel/Lifecycle/Protocol/RestoreClaim/SourceQuery.lean
diagnose "$permit" "$out/permit-service" PermitReadWork.lean PermitReadReturn.lean PermitReadIdentity.lean \
  verification/aeneas/stage1-protocol/restore-work/correct/SandproofRestoreWork/Funs.lean \
  verification/aeneas/stage1-protocol/restore-work/correct/SandproofRestoreWork/Types.lean \
  SandproofRestorePermitFinalize/Funs.lean SandproofRestorePermitFinalize/Types.lean \
  SandproofRestorePermitFinalize/FunsExternal.lean SandproofRestorePermitFinalize/TypesExternal.lean \
  verification/aeneas/create-transaction-link/SandproofSqliteSession/RecordQueryProviders.lean
