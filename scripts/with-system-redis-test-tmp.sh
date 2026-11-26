#!/usr/bin/env bash
# Keep only the Redis test fixture on the manual host's system disk.
set -euo pipefail

if [[ $# == 0 ]]; then
  echo 'usage: bash scripts/with-system-redis-test-tmp.sh COMMAND [ARG ...]' >&2
  exit 2
fi

root_mount="$(findmnt -n -o MAJ:MIN,FSTYPE --target /)"
redis_mount="$(findmnt -n -o MAJ:MIN,FSTYPE --target /var/tmp)"
if [[ "$redis_mount" != "$root_mount" || "$redis_mount" != *" ext4" ]]; then
  echo 'Redis test scratch requires /var/tmp on the ext4 system disk.' >&2
  exit 2
fi

redis_tmp="$(mktemp -d /var/tmp/sandproof-redis-tests.XXXXXXXX)"
cleanup() {
  local status=$?
  trap - EXIT
  if ! rmdir -- "$redis_tmp"; then
    printf 'Redis test scratch remains at %s\n' "$redis_tmp" >&2
    if (( status == 0 )); then status=1; fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

export SANDPROOF_REDIS_TEST_TMPDIR="$redis_tmp"
export SANDPROOF_REDIS_SYSTEM_TMP_ACTIVE=1
printf 'Redis test scratch: %s (%s)\n' "$redis_tmp" "$redis_mount" >&2
"$@"
