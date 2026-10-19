#!/usr/bin/env bash
# Add a compiler output only; leave the original Rust input and options intact.
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
exec bash "$ROOT/scripts/python-entry.sh" "$ROOT/tools/v0_use_inventory.py" rustc "$@"
