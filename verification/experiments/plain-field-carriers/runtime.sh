#!/usr/bin/env bash
# Bind the already registered runtime read-only; no installation or CI copy.
set -euo pipefail
carrier_root="$(realpath -- "$1")"
shift
carrier_target="$(realpath -- "${SANDPROOF_CARRIER_TARGET:?private formal target required}")"
carrier_seed="$(realpath -- "${SANDPROOF_CARRIER_SEED:?existing registered runtime seed required}")"
for carrier_path in "$carrier_root" "$carrier_target" "$carrier_seed"; do
 case "$carrier_path" in /data1/zn/*) ;; *) echo 'Carrier formal storage must be below /data1/zn' >&2; exit 2;; esac
done
carrier_trial="$carrier_seed/src/artifacts/v0-static-tool-trial.IKLrYE8Q"
carrier_fixed=/home/ning/sandproof-proof-337.eZkayQFZ
exec env -u CI -u LD_LIBRARY_PATH -u LD_PRELOAD -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
 bwrap --unshare-user --uid 1062 --gid 1062 --unshare-pid --unshare-ipc \
 --new-session --die-with-parent \
 --ro-bind "$carrier_trial/runtime/build-rootfs" / \
 --proc /proc --dev /dev --ro-bind /sys /sys \
 --ro-bind /usr/bin/taskset /usr/local/bin/proof-taskset \
 --ro-bind /etc/resolv.conf /etc/resolv.conf \
 --ro-bind "$carrier_seed/seed/lean-4.31.0-linux" /home/ning/.local/share/sandproof-proof-runtime/lean-4.31.0-linux \
 --ro-bind "$carrier_seed/seed/sysroot" /home/ning/.local/share/sandproof-proof-runtime/sysroot \
 --ro-bind "$carrier_seed/seed/rust-home/.rustup" /home/ning/.rustup \
 --ro-bind "$carrier_seed/seed/elan" "$carrier_fixed/tools/elan" \
 --ro-bind "$carrier_seed/seed/fixed-tools" "$carrier_fixed/src/.tools" \
 --ro-bind "$carrier_seed/cache/cargo-home" /home/ning/.cargo \
 --ro-bind "$carrier_seed/src" /home/ning/sandproof-proof-v0.yml6x1jB/src \
 --ro-bind "$carrier_trial/runtime/private-home" /home/zn \
 --tmpfs /tmp --tmpfs /run --bind "$carrier_root" /tmp/plain-field-carriers \
 --bind "$carrier_target" /tmp/plain-field-carriers/target-formal \
 --setenv PATH "/home/ning/.local/share/sandproof-proof-runtime/lean-4.31.0-linux/bin:/home/ning/.local/share/sandproof-proof-runtime/sysroot/usr/bin:$carrier_fixed/tools/elan/bin:/home/ning/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
 --setenv ELAN_HOME "$carrier_fixed/tools/elan" --setenv ELAN_TOOLCHAIN sandproof-linux-4.31.0 \
 --setenv RUSTUP_HOME /home/ning/.rustup --setenv RUSTUP_AUTO_INSTALL 0 \
 --setenv CARGO_HOME /tmp/plain-field-carriers/cargo-home --setenv CARGO_NET_OFFLINE true \
 --setenv CARGO_TARGET_DIR /tmp/plain-field-carriers/target-formal \
 --setenv CARGO_BUILD_JOBS 4 --setenv CARGO_INCREMENTAL 0 \
 --setenv TMPDIR /tmp/plain-field-carriers/tmp --setenv OCAMLRUNPARAM o=20 \
 --setenv SANDPROOF_PROOF_TOOL_ROOT "$carrier_fixed/src/.tools" \
 --chdir /tmp/plain-field-carriers/source \
 /bin/bash -c 'umask 077; ulimit -v 67108864; exec proof-taskset -c 0-7 "$@"' bash "$@"
