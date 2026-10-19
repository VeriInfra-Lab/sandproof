#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
# Preserve the caller's private candidate cwd. Formal mode reads evidence and
# cannot start a native test/compiler/linter process through this entry.
bash "$ROOT_DIR/scripts/python-entry.sh" "$ROOT_DIR/tools/proof_native_suite.py" command "$@"
