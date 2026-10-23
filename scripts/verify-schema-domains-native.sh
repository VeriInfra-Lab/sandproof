#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT_DIR"
exec bash scripts/python-entry.sh verification/experiments/schema-domains/native.py "$@"
