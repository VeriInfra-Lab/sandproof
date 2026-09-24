#!/usr/bin/env bash
set -euo pipefail

# Reproduce the exact Rust -> Charon -> Aeneas -> Lean toolchain used by this
# repository.  Everything except the system package manager lives under
# .tools/, which is intentionally ignored by Git.

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
TOOLS_DIR="$ROOT_DIR/.tools"
AENEAS_DIR="$TOOLS_DIR/aeneas"
OPAM_ROOT="$TOOLS_DIR/opam"
OPAM_SWITCH="aeneas-5.3"

AENEAS_REV="505b6ca35217e7be5c96c3e2f8045edfbdf47291"
CHARON_REV="b104e24fea7d721b71e6c39fd70f26ff20bc0980"
OCAML_VERSION="5.3.0"
RUST_NIGHTLY="nightly-2026-08-18"

for command_name in git opam rustup gmake elan; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "missing required command: $command_name" >&2
    exit 1
  fi
done

mkdir -p "$TOOLS_DIR"
rustup toolchain install "$RUST_NIGHTLY" --component rust-src
elan toolchain install "$(tr -d '\n' < "$ROOT_DIR/lean-toolchain")"

if [ ! -d "$AENEAS_DIR/.git" ]; then
  git clone https://github.com/AeneasVerif/aeneas.git "$AENEAS_DIR"
fi
git -C "$AENEAS_DIR" fetch origin "$AENEAS_REV"
git -C "$AENEAS_DIR" checkout --detach "$AENEAS_REV"

if [ ! -d "$AENEAS_DIR/charon/.git" ]; then
  git clone https://github.com/AeneasVerif/charon.git "$AENEAS_DIR/charon"
fi
git -C "$AENEAS_DIR/charon" fetch origin "$CHARON_REV"
git -C "$AENEAS_DIR/charon" checkout --detach "$CHARON_REV"

if ! opam switch list --root "$OPAM_ROOT" --short | grep -Fx "$OPAM_SWITCH" >/dev/null; then
  opam switch create --root "$OPAM_ROOT" "$OPAM_SWITCH" "$OCAML_VERSION" --yes
fi

opam install --root "$OPAM_ROOT" --switch "$OPAM_SWITCH" --yes \
  calendar core_unix domainslib easy_logging menhir ocamlformat.0.27.0 \
  ocamlgraph odoc ppx_deriving ppx_deriving_yojson progress unionFind \
  visitors yojson zarith

opam exec --root "$OPAM_ROOT" --switch "$OPAM_SWITCH" -- \
  gmake -C "$AENEAS_DIR/charon" build-dev
opam exec --root "$OPAM_ROOT" --switch "$OPAM_SWITCH" -- \
  gmake -C "$AENEAS_DIR" build-dev

actual_aeneas_rev="$(git -C "$AENEAS_DIR" rev-parse HEAD)"
actual_charon_rev="$(git -C "$AENEAS_DIR/charon" rev-parse HEAD)"
test "$actual_aeneas_rev" = "$AENEAS_REV"
test "$actual_charon_rev" = "$CHARON_REV"

echo "Aeneas toolchain is ready under $TOOLS_DIR"
