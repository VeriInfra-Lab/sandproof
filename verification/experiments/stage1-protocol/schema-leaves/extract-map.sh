#!/usr/bin/env bash
set -euo pipefail
# Invoked from the isolated project root after the candidate diff is checked.
# The fixed standard Result::map body is generated separately because the
# existing Result::ok model shadows the generated function's unqualified ok.
test ! -e artifacts/schema-map.llbc
mkdir -p verification/aeneas/stage1-protocol/schema-result
.tools/aeneas/charon/bin/charon cargo --preset=aeneas --error-on-warnings \
  --sysroot default --include 'core::result::*::map' \
  --start-from crate::protocol_leaves::result_map \
  --dest-file ../../artifacts/schema-map.llbc -- \
  --manifest-path verification/extract-foundations/Cargo.toml --lib --locked --offline
test -s artifacts/schema-map.llbc
.tools/aeneas/bin/aeneas -backend lean -abort-on-error -warnings-as-errors \
  -split-files -emit-json -namespace SandproofProtocolResultRust \
  -subdir SandproofProtocolResult \
  -dest verification/aeneas/stage1-protocol/schema-result artifacts/schema-map.llbc
