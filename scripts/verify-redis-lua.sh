#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
shasum -a 256 -c verification/redis-lua-inputs.sha256
bash scripts/python-entry.sh verification/experiments/redis-lua/check.py
bash scripts/python-entry.sh tools/proof_native_suite.py require redis-lua
lake build SandboxModel.CenterStore.Lua.Audit
lake env lean SandboxModel/CenterStore/Lua/Audit.lean
bash scripts/python-entry.sh verification/experiments/redis-lua/services.py
