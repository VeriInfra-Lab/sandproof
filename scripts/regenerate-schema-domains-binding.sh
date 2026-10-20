#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
test "$("$CHARON" toolchain-version)" = nightly-2026-08-18
cd "$ROOT_DIR"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
mkdir -p artifacts/schema-domains-binding
PREVIOUS="$(mktemp -d artifacts/schema-domains-binding/previous.XXXXXXXX)"
for kind in request_decode request_encode response_decode response_encode; do
  LLBC="artifacts/schema-domains-binding/$kind.llbc"
  if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/$kind.llbc"; fi
  RUSTFLAGS="--remap-path-prefix=$ROOT_DIR=." \
  "$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
    --targets x86_64-unknown-linux-gnu \
    --start-from "crate::$kind" --include sandproof_protocol \
    --dest-file "../../artifacts/schema-domains-binding/$kind.llbc" -- \
    --manifest-path verification/extract-schema-domains/Cargo.toml --lib --locked --offline
  test -s "$LLBC"
done
# The original backend cannot represent the full Serde trait graph. These
# direct byte calls are conditional B4 bindings, not claimed Aeneas proofs.
# Own guards and ID/counter leaves stay transparent in their actual extraction.
