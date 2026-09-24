#!/usr/bin/env bash
set -euo pipefail
PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
export PATH="/Users/Admin/.elan/bin:$PATH"
export LEAN_LOG_LEVEL=WARNING
# Local proof work needs no external search service or server-managed builds.
exec "$PROJECT_ROOT/.tools/lean-mcp/bin/lean-lsp-mcp" \
  --lean-project-path "$PROJECT_ROOT" \
  --disable-tools lean_leansearch,lean_loogle,lean_leanfinder,lean_state_search,lean_hammer_premise,lean_build \
  "$@"
