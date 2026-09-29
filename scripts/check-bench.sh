#!/usr/bin/env bash
# Joint pinned Harbor/SDK checks; no tasks, VMs, cloud APIs or model calls.
set -euo pipefail
PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

uv run --locked --project benchmarks python -m benchmarks.check_harbor
uv run --locked --project benchmarks \
  python -m unittest discover -s tests/compat -p 'test_*.py' -v

echo "Joint Harbor/SDK imports and loopback wire contracts passed; no benchmark task was executed."
