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
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
LLBC=verification/aeneas/llbc/sandproof_std_mutex.llbc
DEST=verification/aeneas/std-mutex
mkdir -p "$(dirname "$LLBC")" "$DEST" artifacts
PREVIOUS="$(mktemp -d artifacts/std-mutex-extraction.XXXXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --targets x86_64-unknown-linux-gnu \
  --opaque std::sync --opaque tokio --opaque rustix \
  --include sandproof_controller_service \
  --start-from crate::application::diagnostics::_::get_mut \
  --dest-file "$LLBC" -- \
  --manifest-path controller-service/Cargo.toml --lib \
  --features contention-diagnostics --locked --offline
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofStdMutexRust -subdir SandproofStdMutex \
  -dest "$DEST" "$LLBC"
TEMPLATES="$(mktemp -d artifacts/std-mutex-templates.XXXXXXXX)"
for template in FunsExternal_Template.lean TypesExternal_Template.lean; do
  if [[ -f "$DEST/SandproofStdMutex/$template" ]]; then
    mv "$DEST/SandproofStdMutex/$template" "$TEMPLATES/$template"
  fi
done
