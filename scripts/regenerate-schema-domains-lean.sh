#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOL_ROOT="${SANDPROOF_PROOF_TOOL_ROOT:-$ROOT_DIR/.tools}"
CHARON="$TOOL_ROOT/aeneas/charon/bin/charon"
AENEAS="$TOOL_ROOT/aeneas/bin/aeneas"
test "$(git -C "$TOOL_ROOT/aeneas/charon" rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980
test "$(git -C "$TOOL_ROOT/aeneas" rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$("$CHARON" toolchain-version)" = nightly-2026-08-18
cd "$ROOT_DIR"
test -z "${RUSTFLAGS:-}" && test -z "${CARGO_ENCODED_RUSTFLAGS:-}"
LLBC=verification/aeneas/llbc/sandproof_schema_domains.llbc
DEST=verification/aeneas/schema-domains
mkdir -p "$(dirname "$LLBC")" "$DEST" artifacts
PREVIOUS="$(mktemp -d artifacts/schema-domains-extraction.XXXXXXXX)"
if [[ -f "$LLBC" ]]; then mv "$LLBC" "$PREVIOUS/previous.llbc"; fi
"$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  --targets x86_64-unknown-linux-gnu \
  --include sandproof_internal --include sandproof_protocol \
  --start-from crate::status::_::validate \
  --start-from crate::status::_::validate_for \
  --dest-file "$LLBC" -- \
  --manifest-path sandbox-internal/Cargo.toml --lib --locked --offline
test -s "$LLBC"
"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofSchemaDomainsRust -subdir SandproofSchemaDomains \
  -dest "$DEST" "$LLBC"
TEMPLATES="$(mktemp -d artifacts/schema-domains-templates.XXXXXXXX)"
for name in FunsExternal_Template.lean TypesExternal_Template.lean; do
  if [[ -f "$DEST/SandproofSchemaDomains/$name" ]]; then
    mv "$DEST/SandproofSchemaDomains/$name" "$TEMPLATES/$name"
  fi
done
