#!/usr/bin/env bash
# Joint pinned Harbor/SDK checks; no tasks, VMs, cloud APIs or model calls.
set -euo pipefail
PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# Harbor pressure tests use the real Rust reducer through the retention fixture.
# Fetch/build it before entering that fixture's 180-second component deadline.
# Preparation still shares the enclosing job deadline and any failure is fatal.
cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer

uv run --locked --project benchmarks python -m benchmarks.check_harbor
uv run --locked --project benchmarks python -m unittest discover -s benchmarks/tests -v
uv run --locked --project benchmarks \
  python -m unittest discover -s tests/compat -p 'test_*.py' -v

echo "Joint Harbor/SDK imports and loopback wire contracts passed; no benchmark task was executed."
