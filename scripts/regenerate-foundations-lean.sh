#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
cd "$ROOT_DIR"
LLBC=verification/aeneas/llbc/sandproof_foundations.llbc
DEST=verification/aeneas/foundations
mkdir -p "$(dirname "$LLBC")" "$DEST"
# Require a newly emitted input; never translate a leftover from a failed run.
mkdir -p artifacts
PREVIOUS="$(mktemp -d artifacts/foundations-extraction.XXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
# Cargo's driver runs in verification/extract-foundations. The relative
# --dest-file below reaches the same LLBC and is portable across checkouts.
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --start-from crate::counter_from_u64 \
  --start-from crate::counter_next \
  --start-from crate::counter_value \
  --start-from crate::counter_into_u64 \
  --start-from crate::lane_current \
  --start-from crate::store_get_copy \
  --start-from crate::store_put \
  --start-from crate::store_retain_above \
  --start-from crate::option_is_none_or \
  --start-from crate::resource_add \
  --start-from crate::resource_positive \
  --start-from crate::resource_fits \
  --start-from crate::resource_values \
  --start-from crate::create_binding_valid \
  --start-from crate::identity_response \
  --start-from crate::retry_hint \
  --start-from crate::outcome_observation \
  --start-from crate::allocated_number \
  --start-from crate::tenant_from_string \
  --start-from crate::tenant_from_str \
  --start-from crate::tenant_value \
  --start-from crate::tenant_into_string \
  --start-from crate::sandbox_from_string \
  --start-from crate::sandbox_from_str \
  --start-from crate::sandbox_value \
  --start-from crate::sandbox_into_string \
  --start-from crate::operation_from_string \
  --start-from crate::operation_from_str \
  --start-from crate::operation_value \
  --start-from crate::operation_into_string \
  --start-from crate::logical_action_from_string \
  --start-from crate::logical_action_from_str \
  --start-from crate::logical_action_value \
  --start-from crate::logical_action_into_string \
  --start-from crate::resource_from_string \
  --start-from crate::resource_from_str \
  --start-from crate::resource_value \
  --start-from crate::resource_into_string \
  --start-from crate::client_request_from_string \
  --start-from crate::client_request_from_str \
  --start-from crate::client_request_value \
  --start-from crate::client_request_into_string \
  --start-from crate::deployment_from_string \
  --start-from crate::deployment_from_str \
  --start-from crate::deployment_value \
  --start-from crate::deployment_into_string \
  --start-from crate::node_from_string \
  --start-from crate::node_from_str \
  --start-from crate::node_value \
  --start-from crate::node_into_string \
  --start-from crate::center_allocated_number \
  --start-from crate::numeric::seconds_to_milliseconds \
  --start-from crate::numeric::bytes_to_mebibytes \
  --start-from crate::numeric::available_resource \
  --start-from crate::receipts::action_version \
  --start-from crate::receipts::receipt_validate \
  --include sandproof_protocol --include sandproof_center_core \
  --include sandbox_controller_core --dest-file ../aeneas/llbc/sandproof_foundations.llbc \
  -- --manifest-path verification/extract-foundations/Cargo.toml --lib --locked --offline
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofFoundationsRust -subdir SandproofFoundations \
  -dest "$DEST" "$LLBC"
# Editing templates are never accepted as library models or imported proofs.
rm -f "$DEST/SandproofFoundations/FunsExternal_Template.lean" \
  "$DEST/SandproofFoundations/TypesExternal_Template.lean"
