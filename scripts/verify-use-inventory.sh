#!/usr/bin/env bash
# Cheap source admission. Fresh compiler capture is explicit and separately
# reported; this command does not claim to have rerun rustc or proved MIR.
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"
bash scripts/python-entry.sh tools/v0_use_inventory.py admit
