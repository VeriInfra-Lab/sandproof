#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
cd "$ROOT_DIR"
# Charon records this option in the LLBC. Keep it project-relative so moving
# the checkout does not change the source-bound digest; retain the field.
LLBC="verification/aeneas/llbc/sandproof_foundations.llbc"
DEST="$ROOT_DIR/verification/aeneas/foundations"
mkdir -p "$(dirname "$LLBC")" "$DEST"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --start-from crate::counter_from_u64 \
  --start-from crate::counter_next \
  --start-from crate::counter_value \
  --start-from crate::lane_current \
  --start-from crate::store_get_copy \
  --start-from crate::store_put \
  --start-from crate::resource_add \
  --start-from crate::resource_positive \
  --start-from crate::resource_fits \
  --start-from crate::create_binding_valid \
  --start-from crate::identity_response \
  --start-from crate::retry_hint \
  --start-from crate::outcome_observation \
  --include sandproof_protocol --include sandproof_center_core \
  --include sandbox_controller_core --dest-file "$LLBC" \
  -- --manifest-path verification/extract-foundations/Cargo.toml --lib --locked --offline
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofFoundationsRust -subdir SandproofFoundations \
  -dest "$DEST" "$LLBC"
# Editing templates are never accepted as library models or imported proofs.
rm -f "$DEST/SandproofFoundations/FunsExternal_Template.lean" \
  "$DEST/SandproofFoundations/TypesExternal_Template.lean"
