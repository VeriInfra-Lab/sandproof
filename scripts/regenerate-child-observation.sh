#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
export PYTHONPATH="$root${PYTHONPATH:+:$PYTHONPATH}"
exec bash "$root/scripts/python-entry.sh" "$root/verification/experiments/child-observation/replay.py" "$@"
