#!/usr/bin/env bash
# Inherit the already fixed runtime. Only this variant and its stable target are writable.
set -euo pipefail
writer_variant="$(realpath -- "$1")"
shift
writer_target="$(realpath -- "${SANDPROOF_WRITER_TARGET:?leased target required}")"
exec env -u CI -u LD_LIBRARY_PATH -u LD_PRELOAD -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
 "${SANDPROOF_PROOF_BWRAP:?measured existing proof-bwrap alias required}" \
 --unshare-user --uid 1062 --gid 1062 --unshare-pid --unshare-ipc \
 --new-session --die-with-parent --ro-bind / / \
 --proc /proc --dev /dev --tmpfs /tmp --tmpfs /run \
 --bind "$writer_variant" /tmp/bounded-writer \
 --bind "$writer_target" /tmp/bounded-writer/target-formal \
 --setenv PATH "/home/ning/.local/share/sandproof-proof-runtime/lean-4.31.0-linux/bin:/home/ning/.local/share/sandproof-proof-runtime/sysroot/usr/bin:/home/ning/sandproof-proof-337.eZkayQFZ/tools/elan/bin:/home/ning/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
 --setenv ELAN_HOME /home/ning/sandproof-proof-337.eZkayQFZ/tools/elan \
 --setenv ELAN_TOOLCHAIN sandproof-linux-4.31.0 \
 --setenv RUSTUP_HOME /home/ning/.rustup --setenv RUSTUP_AUTO_INSTALL 0 \
 --setenv CARGO_HOME /tmp/bounded-writer/cargo-home --setenv CARGO_NET_OFFLINE true \
 --setenv CARGO_TARGET_DIR /tmp/bounded-writer/target-formal \
 --setenv CARGO_BUILD_JOBS 4 --setenv CARGO_INCREMENTAL 0 \
 --setenv TMPDIR /tmp/bounded-writer/tmp --setenv OCAMLRUNPARAM o=20 \
 --setenv SANDPROOF_PROOF_TOOL_ROOT /home/ning/sandproof-proof-337.eZkayQFZ/src/.tools \
 --chdir /tmp/bounded-writer/source \
 /bin/bash -c 'umask 077; ulimit -v 67108864; exec proof-taskset -c 0-7 "$@"' bash "$@"
