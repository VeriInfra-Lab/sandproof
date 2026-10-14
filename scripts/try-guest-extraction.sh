#!/usr/bin/env bash
# Isolated feasibility trial, not the controller proof acceptance workflow.
set -euo pipefail

PROJECT_ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$PROJECT_ROOT"
CHARON="$PROJECT_ROOT/.tools/aeneas/charon/bin/charon"
AENEAS="$PROJECT_ROOT/.tools/aeneas/bin/aeneas"

test -x "$CHARON" && test -x "$AENEAS"
test "$(git -C .tools/aeneas rev-parse HEAD)" = 505b6ca35217e7be5c96c3e2f8045edfbdf47291
test "$(git -C .tools/aeneas/charon rev-parse HEAD)" = b104e24fea7d721b71e6c39fd70f26ff20bc0980

mkdir -p artifacts/guest-extraction
TRIAL_DIR="$(mktemp -d "$PROJECT_ROOT/artifacts/guest-extraction/trial.XXXXXX")"
SOURCE_FILES=(Cargo.toml Cargo.lock rust-toolchain.toml sandbox-guest-core/Cargo.toml sandbox-guest-core/src/*.rs)
sha256sum "${SOURCE_FILES[@]}" > "$TRIAL_DIR/source-before.sha256"
git rev-parse HEAD > "$TRIAL_DIR/base-commit.txt"

ROOTS=(
  crate::command::CommandStream::new
  crate::command::CommandStream::queue_output
  crate::command::CommandStream::sent_output
  crate::command::CommandStream::observe_eof
  crate::command::CommandStream::observe_exit
  crate::command::CommandStream::fail
  crate::command::CommandStream::outcome
  crate::transfer::Transfer::new
  crate::transfer::Transfer::receive
  crate::transfer::Transfer::record_write
  crate::transfer::Transfer::end_input
  crate::transfer::Transfer::fail
  crate::transfer::Transfer::finish
  crate::transfer::Transfer::outcome
)
ROOT_ARGS=()
for guest_root in "${ROOTS[@]}"; do
  # In this fixed Charon, the final dependency pass cannot match inherent
  # impl blocks by type name. Match that single block component with a glob;
  # audit the exact translated Rust method names below, not just exit status.
  guest_pattern="${guest_root/::CommandStream::/::_::}"
  guest_pattern="${guest_pattern/::Transfer::/::_::}"
  ROOT_ARGS+=(--start-from "$guest_pattern")
done
printf '%s\n' "${ROOTS[@]}" > "$TRIAL_DIR/roots.txt"

# Separate Cargo output avoids modifying any concurrently checked host helper.
CARGO_TARGET_DIR="$PROJECT_ROOT/artifacts/guest-extraction/target" \
  "$CHARON" cargo --preset=aeneas --error-on-warnings --sysroot default \
  "${ROOT_ARGS[@]}" --dest-file "$TRIAL_DIR/sandproof_guest_core.llbc" -- \
  --manifest-path sandbox-guest-core/Cargo.toml --lib --locked --offline \
  > "$TRIAL_DIR/charon.log" 2>&1

"$AENEAS" -backend lean -abort-on-error -warnings-as-errors -split-files \
  -emit-json -namespace SandproofGuestRust -subdir SandproofGuestCore \
  -dest "$TRIAL_DIR" "$TRIAL_DIR/sandproof_guest_core.llbc" \
  > "$TRIAL_DIR/aeneas.log" 2>&1

bash scripts/python-entry.sh tools/check_guest_extraction.py \
  "$TRIAL_DIR/roots.txt" "$TRIAL_DIR/translation.json" \
  | tee "$TRIAL_DIR/inventory-check.txt"

sha256sum "${SOURCE_FILES[@]}" > "$TRIAL_DIR/source-after.sha256"
cmp "$TRIAL_DIR/source-before.sha256" "$TRIAL_DIR/source-after.sha256"

echo "Guest Rust extraction trial emitted to $TRIAL_DIR"
echo 'Inspect translation.json for opaque/external dependencies before designing proofs.'
echo 'Generated files, including any external templates, stay outside the proof library.'
echo 'This does not compile Lean, prove properties, or accept a running guest service.'
