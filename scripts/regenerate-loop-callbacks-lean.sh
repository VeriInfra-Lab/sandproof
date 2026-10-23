#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
test "$("$CHARON" toolchain-version)" = nightly-2026-08-18
cd "$ROOT_DIR"
LLBC=verification/aeneas/llbc/sandproof_loop_callbacks.llbc
DEST=verification/aeneas/loop-callbacks
mkdir -p "$(dirname "$LLBC")" "$DEST" artifacts
PREVIOUS="$(mktemp -d artifacts/loop-callbacks-extraction.XXXXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --start-from crate::lookup --start-from crate::contains \
  --start-from crate::lookup_u64 --start-from crate::contains_u64 \
  --include sandbox_controller_core \
  --dest-file ../aeneas/llbc/sandproof_loop_callbacks.llbc \
  -- --manifest-path verification/extract-loop-callbacks/Cargo.toml --lib --locked --offline
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofLoopCallbacksRust -subdir SandproofLoopCallbacks \
  -dest "$DEST" "$LLBC"
TEMPLATES="$(mktemp -d artifacts/loop-callbacks-templates.XXXXXXXX)"
for template in FunsExternal_Template.lean TypesExternal_Template.lean; do
  if [[ -f "$DEST/SandproofLoopCallbacks/$template" ]]; then
    mv "$DEST/SandproofLoopCallbacks/$template" "$TEMPLATES/$template"
  fi
done
