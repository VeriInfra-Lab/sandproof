#!/usr/bin/env bash
set -euo pipefail
umask 077

# Diagnostic workflow only. The formal PR keeps the normal full CI.
: "${RUNNER_TEMP:?}"
: "${TMPDIR:?}"
export SANDPROOF_REDIS_SERVER
SANDPROOF_REDIS_SERVER="$(python3 tools/ci_redis.py check)"
export CARGO_NET_OFFLINE=true

if [[ "$(findmnt -n -o FSTYPE --target "$TMPDIR")" != xfs ||
      "$(findmnt -n -o FSTYPE --target /tmp)" != ext4 ]]; then
  printf 'Unexpected guest volumes, no comparison was performed.\n' >&2
  findmnt -n -o TARGET,FSTYPE --target "$TMPDIR"
  findmnt -n -o TARGET,FSTYPE --target /tmp
  exit 2
fi

xfs=$(mktemp -d "$TMPDIR/v3-cas.XXXXXXXX")
ext4=$(mktemp -d /tmp/v3-cas.XXXXXXXX)
trap 'rmdir "$xfs" "$ext4"' EXIT
for disk in "$xfs" "$ext4"; do
  findmnt -n -o TARGET,FSTYPE --target "$disk"
done
cargo test --locked --offline -p sandproof-center-service --features redis-tests \
  --test business_authority --no-run --message-format=json > "$RUNNER_TEMP/v3-cas-build.jsonl"
binary=$(python3 - "$RUNNER_TEMP/v3-cas-build.jsonl" <<'PY'
import json
import sys
from pathlib import Path

rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
matches = [row["executable"] for row in rows if row.get("reason") == "compiler-artifact"
           and row["target"]["name"] == "business_authority" and row.get("executable")]
assert len(matches) == 1, matches
print(Path(matches[0]).resolve(strict=True))
PY
)
test -x "$binary"
sha256sum "$binary"
status=0
attempt=0
# Reverse the order in the second pair. Both paths use identical Redis AOF,
# fsync configuration, assertions, timeouts, CPU allocation and test binary.
for medium in xfs ext4 ext4 xfs; do
  attempt=$((attempt + 1))
  dir=${!medium}
  printf 'cas_probe begin attempt=%s medium=%s\n' "$attempt" "$medium"
  rc=0
  TMPDIR="$dir" timeout 100s "$binary" || rc=$?
  printf 'cas_probe end attempt=%s medium=%s status=%s\n' "$attempt" "$medium" "$rc"
  if (( rc != 0 )); then status=1; fi
done
exit "$status"
