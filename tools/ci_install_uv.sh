#!/usr/bin/env bash
# Trusted image preparation only. Download one official wheel, never execute pip.
set -euo pipefail
if [[ "$(id -u)" != 0 ]]; then
  printf 'uv image installation requires root\n' >&2
  exit 1
fi
pins_file="${1:-/root/ci-pins.json}"
# Validate the immutable release and public source before making any request.
pin_values="$(python3 - "$pins_file" <<'PY'
import json
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit

pins = json.loads(Path(sys.argv[1]).read_text())
version, url, digest = (pins.get(name) for name in ('uv_version', 'uv_url', 'uv_sha256'))
filename = 'uv-0.11.31-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl'
if version != '0.11.31' or not isinstance(url, str) or not isinstance(digest, str):
    raise ValueError('uv requires fixed release 0.11.31 and artifact pins')
source = urlsplit(url)
if (source.scheme != 'https' or source.netloc != 'files.pythonhosted.org'
        or source.query or source.fragment
        or not re.fullmatch(r'/packages/[a-f0-9]{2}/[a-f0-9]{2}/[a-f0-9]{60}/' + re.escape(filename), source.path)
        or not re.fullmatch(r'[a-f0-9]{64}', digest)):
    raise ValueError('uv artifact must be a SHA256-pinned official PyPI wheel')
print(url)
print(digest)
PY
)"
uv_url="${pin_values%%$'\n'*}"
uv_sha="${pin_values##*$'\n'}"
uv_stage="$(mktemp -d /tmp/sandproof-ci-uv.XXXXXXXX)"
cleanup_uv() {
  # These are the only two files this invocation creates in its private stage.
  rm -f -- "$uv_stage/uv.whl" "$uv_stage/uv"
  rmdir -- "$uv_stage"
}
trap cleanup_uv EXIT
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 "$uv_url" -o "$uv_stage/uv.whl"
printf '%s  %s\n' "$uv_sha" "$uv_stage/uv.whl" | sha256sum -c -
python3 - "$uv_stage/uv.whl" "$uv_stage/uv" <<'PY'
from pathlib import Path
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as wheel:
    # Extract only this member, never paths or install hooks from the archive.
    Path(sys.argv[2]).write_bytes(wheel.read('uv-0.11.31.data/scripts/uv'))
PY
chmod 0500 "$uv_stage/uv"
uv_version="$("$uv_stage/uv" --version)"
uv_version_pattern='^uv 0[.]11[.]31( [(][^()[:cntrl:]]+[)])?$'
if [[ ! "$uv_version" =~ $uv_version_pattern ]]; then
  printf 'Pinned uv wheel returned unexpected version: %s\n' "$uv_version" >&2
  exit 1
fi
install -o root -g root -m 0555 "$uv_stage/uv" /usr/local/bin/uv
/usr/local/bin/uv --version
