#!/usr/bin/env bash
# Reuse the already registered a6000 runtime; this installs no tools or CI.
set -euo pipefail
writer_physical_root="$(realpath -- "$1")"
shift
writer_target="$(realpath -- "${SANDPROOF_WRITER_TARGET:?allocated private stable formal target required}")"
writer_seed="$(realpath -- "${SANDPROOF_WRITER_SEED:?registered fixed runtime seed required}")"
case "$writer_physical_root" in /data1/zn/*) ;; *) echo 'Writer source/evidence must be under /data1/zn' >&2; exit 2;; esac
case "$writer_target" in /data1/zn/*) ;; *) echo 'Writer compiler target must be under /data1/zn' >&2; exit 2;; esac
case "$writer_seed" in /data1/zn/*) ;; *) echo 'Use the existing data1 runtime seed' >&2; exit 2;; esac
writer_trial="$writer_seed/src/artifacts/v0-static-tool-trial.IKLrYE8Q"
writer_virtual_src=/home/ning/sandproof-proof-v0.yml6x1jB/src
writer_virtual_fixed=/home/ning/sandproof-proof-337.eZkayQFZ
exec env -u CI -u LD_LIBRARY_PATH -u LD_PRELOAD -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
 bwrap --unshare-user --uid 1062 --gid 1062 --unshare-pid --unshare-ipc \
 --new-session --die-with-parent \
 --ro-bind "$writer_trial/runtime/build-rootfs" / \
 --proc /proc --dev /dev --ro-bind /sys /sys \
 --ro-bind /usr/bin/taskset /usr/local/bin/proof-taskset \
 --ro-bind /etc/resolv.conf /etc/resolv.conf \
 --ro-bind "$writer_seed/seed/lean-4.31.0-linux" /home/ning/.local/share/sandproof-proof-runtime/lean-4.31.0-linux \
 --ro-bind "$writer_seed/seed/sysroot" /home/ning/.local/share/sandproof-proof-runtime/sysroot \
 --ro-bind "$writer_seed/seed/rust-home/.rustup" /home/ning/.rustup \
 --ro-bind "$writer_seed/seed/elan" "$writer_virtual_fixed/tools/elan" \
 --ro-bind "$writer_seed/seed/fixed-tools" "$writer_virtual_fixed/src/.tools" \
 --ro-bind "$writer_seed/cache/cargo-home" /home/ning/.cargo \
 --ro-bind "$writer_seed/src" "$writer_virtual_src" \
 --ro-bind "$writer_trial/runtime/private-home" /home/zn \
 --tmpfs /tmp --tmpfs /run --bind "$writer_physical_root" /tmp/bounded-writer \
 --bind "$writer_target" /tmp/bounded-writer/target-formal \
 --setenv PATH "/home/ning/.local/share/sandproof-proof-runtime/lean-4.31.0-linux/bin:/home/ning/.local/share/sandproof-proof-runtime/sysroot/usr/bin:$writer_virtual_fixed/tools/elan/bin:/home/ning/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
 --setenv ELAN_HOME "$writer_virtual_fixed/tools/elan" \
 --setenv ELAN_TOOLCHAIN sandproof-linux-4.31.0 \
 --setenv RUSTUP_HOME /home/ning/.rustup --setenv RUSTUP_AUTO_INSTALL 0 \
 --setenv CARGO_HOME /tmp/bounded-writer/cargo-home --setenv CARGO_NET_OFFLINE true \
 --setenv CARGO_TARGET_DIR /tmp/bounded-writer/target-formal \
 --setenv CARGO_BUILD_JOBS 4 --setenv CARGO_INCREMENTAL 0 \
 --setenv TMPDIR /tmp/bounded-writer/tmp --setenv OCAMLRUNPARAM o=20 \
 --setenv SANDPROOF_PROOF_TOOL_ROOT "$writer_virtual_fixed/src/.tools" \
 --chdir /tmp/bounded-writer/source \
 /bin/bash -c 'umask 077; ulimit -v 67108864; exec proof-taskset -c 0-7 "$@"' bash "$@"
