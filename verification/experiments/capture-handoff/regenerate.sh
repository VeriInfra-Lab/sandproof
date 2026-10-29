#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
cd "$ROOT"
TOOLS=${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT/.tools}
CHARON="$TOOLS/aeneas/charon/bin/charon"
AENEAS="$TOOLS/aeneas/bin/aeneas"
variant=${1:-values}
case "$variant" in
  values)
    roots=(accepted dispatch register observe pause_failure terminate stop_permit registration_allowed commit eligibility material_uses)
    ;;
  handler) roots=(compensate) ;;
  worker) roots=(worker) ;;
  *) echo 'expected values, handler or worker' >&2; exit 2 ;;
esac
out="${SANDPROOF_CAPTURE_OUTPUT:?set a fresh private output directory}"
mkdir -m 700 "$out"
args=()
for root in "${roots[@]}"; do args+=(--start-from "crate::$root"); done
cd verification/extract-capture-handoff
RUSTFLAGS="--remap-path-prefix=$ROOT=." "$CHARON" cargo --preset=aeneas \
  --error-on-warnings --sysroot default "${args[@]}" --include sandbox_controller_core \
  --dest-file "$out/capture_handoff.llbc" -- --manifest-path Cargo.toml --lib --locked --offline
cd "$ROOT"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
  -namespace CaptureHandoffRust -subdir CaptureHandoff \
  -dest "$out/lean" "$out/capture_handoff.llbc"
