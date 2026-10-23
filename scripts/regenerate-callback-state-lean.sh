#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
case "${1:-correct}" in
  correct) suffix=correct ;;
  no-write-toggle) suffix=no_write_toggle ;;
  double-record) suffix=double_record ;;
  *) echo 'Unknown callback-state generation variant' >&2; exit 2 ;;
esac
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
test "$("$CHARON" toolchain-version)" = nightly-2026-08-18
cd "$ROOT_DIR"
LLBC=verification/aeneas/llbc/sandproof_callback_state.llbc
RAW="verification/aeneas/llbc/sandproof_callback_state_$suffix.llbc"
DEST=verification/aeneas/callback-state
mkdir -p "$(dirname "$LLBC")" "$DEST" artifacts
PREVIOUS="$(mktemp -d artifacts/callback-state-extraction.XXXXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
if [[ -f "$RAW" ]]; then mv "$RAW" "$PREVIOUS/previous-raw.llbc"; fi
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --start-from crate::toggle_step --start-from crate::trace_step \
  --include sandbox_controller_core \
  --dest-file "../aeneas/llbc/sandproof_callback_state_$suffix.llbc" \
  -- --manifest-path verification/extract-callback-state/Cargo.toml --lib --locked --offline
test -s "$RAW"
mv "$RAW" "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofCallbackStateRust -subdir SandproofCallbackState \
  -dest "$DEST" "$LLBC"
test ! -e "$DEST/SandproofCallbackState/FunsExternal_Template.lean"
test ! -e "$DEST/SandproofCallbackState/TypesExternal_Template.lean"
