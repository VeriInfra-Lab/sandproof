#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
test "$("$CHARON" toolchain-version)" = nightly-2026-08-18
cd "$ROOT_DIR"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
LLBC=verification/aeneas/llbc/sandproof_data_drain.llbc
DEST=verification/aeneas/data-drain
mkdir -p "$(dirname "$LLBC")" "$DEST" artifacts
PREVIOUS="$(mktemp -d artifacts/data-drain-extraction.XXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
"$CHARON" rustc --preset=aeneas --error-on-warnings --sysroot default \
  --targets x86_64-unknown-linux-gnu \
  --start-from crate::read_drained --start-from crate::read_routes_dropped \
  --include crate --dest-file "$LLBC" \
  -- --edition 2024 --crate-type lib --crate-name sandproof_data_drain_extract \
  verification/extract-data-drain/src/lib.rs
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofDataDrainRust -subdir SandproofDataDrain \
  -dest "$DEST" "$LLBC"
# The templates are retained as diagnostics, never imported as evidence.
TEMPLATES="$(mktemp -d artifacts/data-drain-templates.XXXXXX)"
for template in FunsExternal_Template.lean TypesExternal_Template.lean; do
  if [[ -f "$DEST/SandproofDataDrain/$template" ]]; then
    mv "$DEST/SandproofDataDrain/$template" "$TEMPLATES/$template"
  fi
done
