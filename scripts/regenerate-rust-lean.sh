#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
CHARON="$ROOT_DIR/.tools/aeneas/charon/bin/charon"
AENEAS="$ROOT_DIR/.tools/aeneas/bin/aeneas"
LLBC_DIR="$ROOT_DIR/verification/aeneas/llbc"
LEAN_DIR="$ROOT_DIR/verification/aeneas/generated"
LLBC_FILE="$LLBC_DIR/sandbox_controller_core.llbc"

AENEAS_REV="505b6ca35217e7be5c96c3e2f8045edfbdf47291"
CHARON_REV="b104e24fea7d721b71e6c39fd70f26ff20bc0980"

if [ ! -x "$CHARON" ] || [ ! -x "$AENEAS" ]; then
  echo "Aeneas is not bootstrapped; run scripts/bootstrap-aeneas.sh first" >&2
  exit 1
fi

test "$(git -C "$ROOT_DIR/.tools/aeneas" rev-parse HEAD)" = "$AENEAS_REV"
test "$(git -C "$ROOT_DIR/.tools/aeneas/charon" rev-parse HEAD)" = "$CHARON_REV"

mkdir -p "$LLBC_DIR" "$LEAN_DIR"

"$CHARON" cargo \
  --preset=aeneas \
  --error-on-warnings \
  --sysroot default \
  --start-from crate::request::handle_create \
  --start-from crate::controller::handle_controller \
  --start-from crate::model::initial_world \
  --start-from crate::request::handle_pause \
  --start-from crate::request::handle_resume \
  --start-from crate::request::handle_kill \
  --start-from crate::request::handle \
  --start-from crate::auxiliary::handle_rpc_timeout \
  --start-from crate::auxiliary::handle_access \
  --start-from crate::effect::handle_effect \
  --start-from crate::scheduler::handle_reserve_startup \
  --start-from crate::scheduler::handle_reserve_pause_snapshot \
  --start-from crate::scheduler::handle_dispatch_prepare \
  --start-from crate::scheduler::handle_schedule_seal_snapshot \
  --start-from crate::scheduler::handle_schedule_commit_snapshot \
  --start-from crate::scheduler::handle_schedule_stop_vm \
  --start-from crate::scheduler::handle_schedule_cleanup_vm \
  --start-from crate::scheduler::handle_schedule_cleanup_slot \
  --start-from crate::scheduler::handle_scheduler \
  --start-from crate::worker::handle_worker \
  --start-from crate::publication::handle_publication \
  --start-from crate::recovery_monitor::handle_runtime \
  --start-from crate::recovery_monitor::view \
  --start-from crate::recovery_monitor::monitor \
  --start-from crate::transition::reserve_startup_resources \
  --start-from crate::transition::reserve_pause_snapshot \
  --dest-file "$LLBC_FILE" \
  -- \
  --manifest-path "$ROOT_DIR/controller-core/Cargo.toml"

"$AENEAS" \
  -backend lean \
  -abort-on-error \
  -warnings-as-errors \
  -split-files \
  -emit-json \
  -namespace SandboxControllerRust \
  -subdir SandboxControllerCore \
  -dest "$LEAN_DIR" \
  "$LLBC_FILE"

# Aeneas emits an axiom-based editing template.  The repository instead keeps
# a total, hand-written FunsExternal.lean beside it, so the template must never
# be mistaken for part of the verified source tree.
rm -f "$LEAN_DIR/SandboxControllerCore/FunsExternal_Template.lean"

echo "Regenerated Rust semantics under $LEAN_DIR/SandboxControllerCore"
