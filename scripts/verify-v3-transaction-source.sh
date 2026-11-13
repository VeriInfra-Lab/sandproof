#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$root"
shasum -a 256 -c verification/v3-transaction-source-inputs.sha256
if [[ "$#" != 1 ]]; then
    echo 'usage: bash scripts/verify-v3-transaction-source.sh <new-evidence-dir>' >&2
    exit 2
fi
bash scripts/python-entry.sh verification/experiments/stage1-protocol/v3-transaction-source.py \
    --scope stage1-v3-transaction-source --output "$1"
