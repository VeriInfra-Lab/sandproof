#!/usr/bin/env bash
# Run the V3 public-application test with the existing cfg(test) checkpoint hook.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo "usage: $0 NEW_OUTPUT_DIRECTORY" >&2
  exit 2
fi

umask 077
root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
head=$(git -C "$root" rev-parse HEAD)
mkdir -- "$1"
output=$(cd -- "$1" && pwd -P)
source_dir="$output/source"
pkg=verification/experiments/stage1-protocol/coordinator-revocation
suite=controller-service/src/controller/checkpoint/tests.rs
test_name=controller::checkpoint::tests::unload_revocation::maintenance_commit_uncertainty_revokes_both_routes_before_unload_is_recorded

git clone --quiet --shared --no-checkout "$root" "$source_dir"
cd "$source_dir"
git switch --quiet --detach "$head"
git show --no-patch --format='%H %T' HEAD > "$output/source.txt"
git apply --index "$pkg/unload-maintenance-watch-registration-candidate.patch"
test "$(git diff --cached --name-only)" = "$suite"
git diff --cached --check
git diff --cached -- "$suite" > "$output/test-registration.diff"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$output/target}"
export CARGO_TERM_COLOR=never
cargo --config scripts/check-profiles.toml test --locked --offline \
  -p sandproof-controller-service --lib "$test_name" -- --exact --test-threads=1 \
  2>&1 | tee "$output/cargo.log"
grep -Fxq "test $test_name ... ok" "$output/cargo.log"
grep -Eq '^test result: ok\. 1 passed; 0 failed; 0 ignored;' "$output/cargo.log"
