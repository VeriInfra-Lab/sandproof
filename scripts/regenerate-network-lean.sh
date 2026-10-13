#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
cd "$ROOT_DIR"
LLBC="$ROOT_DIR/verification/aeneas/llbc/sandproof_network.llbc"
DEST="$ROOT_DIR/verification/aeneas/network"
mkdir -p "$(dirname "$LLBC")" "$DEST"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --start-from crate::cidr_contains \
  --start-from crate::protected_ipv4 \
  --start-from crate::destination_allowed \
  --start-from crate::parse_ipv4_bits \
  --start-from crate::egress \
  --start-from crate::ingress \
  --start-from crate::clone_world \
  --start-from crate::initial_world \
  --include sandbox_controller_core --dest-file "$LLBC" \
  -- --manifest-path verification/extract-network/Cargo.toml --lib --locked --offline
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofNetworkRust -subdir SandproofNetwork \
  -dest "$DEST" "$LLBC"
# Editing templates are not library models and never become proof assumptions.
rm -f "$DEST/SandproofNetwork/FunsExternal_Template.lean" \
  "$DEST/SandproofNetwork/TypesExternal_Template.lean"
