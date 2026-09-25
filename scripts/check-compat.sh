#!/usr/bin/env bash
# Pinned E2B Python SDK against synthetic loopback responses only; no VM/cloud.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

uv run --locked --project tests/compat \
  python -m unittest discover -s tests/compat -p 'test_*.py' -v

echo "SDK contract checks passed against synthetic loopback responses only."
