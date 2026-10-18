#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
cd "$ROOT_DIR"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
mkdir -p artifacts/json-request-binding
PREVIOUS="$(mktemp -d artifacts/json-request-binding/previous.XXXXXX)"
for kind in decode encode; do
  LLBC="artifacts/json-request-binding/$kind.llbc"
  if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/$kind.llbc"; fi
  RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
  "$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
    --start-from "crate::json_request::$kind" --include sandproof_protocol \
    --dest-file "../../artifacts/json-request-binding/$kind.llbc" \
    -- --manifest-path verification/extract-foundations/Cargo.toml --lib --locked --offline
  test -s "$LLBC"
done
# No Aeneas generation is claimed for these two byte wrappers: the original
# backend cannot represent the complete fixed Serde trait graph. Their exact
# calls are reviewed B4 bindings; own guards/leaves use the normal V0 generation.
