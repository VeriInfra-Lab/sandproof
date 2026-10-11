#!/usr/bin/env bash
# One interpreter choice for the unvendored development tools. SDK/Harbor use
# their locked uv projects, with the same supported Python version range.
set -euo pipefail
if [[ -n "${SANDPROOF_PYTHON:-}" ]]; then
  tool_python="$SANDPROOF_PYTHON"
else
  tool_python=''
  for candidate in python3.13 python3.12 python3; do
    if command -v "$candidate" >/dev/null 2>&1; then
      tool_python="$candidate"
      break
    fi
  done
fi
if [[ -z "$tool_python" ]]; then
  echo 'Python >=3.12,<3.14 required; set SANDPROOF_PYTHON to its executable.' >&2
  exit 2
fi
"$tool_python" -c 'import sys; sys.exit(0 if (3,12) <= sys.version_info[:2] < (3,14) else "Python >=3.12,<3.14 required; set SANDPROOF_PYTHON.")'
exec "$tool_python" "$@"
