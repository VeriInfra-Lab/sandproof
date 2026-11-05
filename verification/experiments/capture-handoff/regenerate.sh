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
  caller)
    roots=('capture::*::accepted' 'capture::*::registration_dispatched'
           'capture::*::register_bundle' 'capture::*::observe_bundle'
           'capture::*::ordinary_failure' 'capture::*::terminate'
           'capture::*::may_stop_original' 'capture::*::accepts_registration'
           'capture::*::commit' 'capture::*::eligibility'
           'capture::*::has_capture_work' 'capture::*::has_unload_source'
           request::handle_compensation worker::late_capture::record
           publication::handle_publish_paused worker::pause_stop::record
           effect::dispatch_stop effect::acknowledge_effect
           'maintenance::released_execution::*::command' effect::receipt_pause)
    ;;
  *) echo 'expected values, handler, worker or caller' >&2; exit 2 ;;
esac
out="${SANDPROOF_CAPTURE_OUTPUT:?set a fresh private output directory}"
if [[ "$variant" == caller && -n "${SANDPROOF_PAUSE_RECEIPT_CORRECT_CAPTURE:-}" ]]; then
  # This finite receipt batch already captured the same twenty real roots.
  # Stage only after the original source/output admission; old callers still
  # perform their own exact generation, model and proof checks below replay.
  expected_roots=()
  for root in "${roots[@]}"; do expected_roots+=(--expected-root "$root"); done
  bash scripts/python-entry.sh verification/experiments/pause-receipt/capture.py \
    --source "$ROOT" --origin "$SANDPROOF_PAUSE_RECEIPT_CORRECT_CAPTURE" --output "$out" \
    "${expected_roots[@]}"
  exit 0
fi
mkdir -m 700 "$out"
args=()
for root in "${roots[@]}"; do args+=(--start-from "crate::$root"); done
namespace=CaptureHandoffRust
subdir=CaptureHandoff
crate=verification/extract-capture-handoff
if [[ "$variant" == worker || "$variant" == caller ]]; then
  args+=(--include 'core::option::*::and_then')
fi
if [[ "$variant" == caller ]]; then
  args+=(--include 'core::option::*::as_ref')
  # Aeneas filters uncalled external top-level roots. Extract the real local
  # crate so late_capture::record survives into the final Lean, not just LLBC.
  crate=controller-core
  namespace=CaptureCallerRust.sandbox_controller_core
  subdir=CaptureCaller
fi
cd "$crate"
RUSTFLAGS="--remap-path-prefix=$ROOT=." "$CHARON" cargo --preset=aeneas \
  --error-on-warnings --sysroot default "${args[@]}" --include sandbox_controller_core \
  --dest-file "$out/capture_handoff.llbc" -- --manifest-path Cargo.toml --lib --locked --offline
cd "$ROOT"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files -emit-json \
  -namespace "$namespace" -subdir "$subdir" \
  -dest "$out/lean" "$out/capture_handoff.llbc"
