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
LLBC=verification/aeneas/llbc/sandproof_first_chain.llbc
DEST=verification/aeneas/first-chain
mkdir -p "$(dirname "$LLBC")" "$DEST"
mkdir -p artifacts
PREVIOUS="$(mktemp -d artifacts/first-chain-extraction.XXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
EXTRACTION_SYSROOT="$("$CHARON" toolchain-path)"
CARGO_CACHE="${CARGO_HOME:-$HOME/.cargo}"
# Source locations are meaningful, but private host/checkout prefixes are not.
# Freeze these explicit remaps in the first-chain environment, not in V0.
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=. --remap-path-prefix=$EXTRACTION_SYSROOT=<extraction-sysroot> --remap-path-prefix=$CARGO_CACHE=<cargo-cache>" \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --targets x86_64-unknown-linux-gnu \
  --start-from crate::identity_response --start-from crate::lifecycle_response \
  --start-from crate::operation_response \
  --include sandproof_protocol --include sandproof_center_core \
  --include 'core::option::_::eq' --dest-file "$LLBC" \
  -- --manifest-path verification/extract-first-chain/Cargo.toml --lib --locked --offline
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofFirstChainRust -subdir SandproofFirstChain \
  -dest "$DEST" "$LLBC"
# Templates are diagnostics, not imported evidence or accepted assumptions.
mkdir -p artifacts
TEMPLATES="$(mktemp -d artifacts/first-chain-templates.XXXXXX)"
for template in FunsExternal_Template.lean TypesExternal_Template.lean; do
  if [[ -f "$DEST/SandproofFirstChain/$template" ]]; then
    mv "$DEST/SandproofFirstChain/$template" "$TEMPLATES/$template"
  fi
done
