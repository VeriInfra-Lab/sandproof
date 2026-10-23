#!/usr/bin/env bash
# Cheap source admission. Fresh compiler capture is explicit and separately
# reported; this command does not claim to have rerun rustc or proved MIR.
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"
bash scripts/python-entry.sh tools/v0_use_inventory.py admit
bash scripts/python-entry.sh -m tools.check_os_boundary_catalogue
bash scripts/python-entry.sh -m tools.check_tls_certificate_binding
bash scripts/python-entry.sh -m tools.check_shared_field_bindings

bash scripts/python-entry.sh tools/check_btree_set_binding.py
bash scripts/python-entry.sh verification/experiments/protected-directory/check.py
bash scripts/python-entry.sh verification/experiments/tokio-mutex/check.py
