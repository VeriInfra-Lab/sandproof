#!/bin/bash
# Guest-only repair for the two pinned Bullseye QEMU benchmark images.
set -eu
. /etc/os-release
test "$ID" = debian
test "$VERSION_CODENAME" = bullseye
test -s /usr/share/keyrings/debian-archive-keyring.gpg
test -f /app/alpine.iso
test -f /app/alpine-disk.qcow2

# Reject an unexpected source layout rather than silently mixing repositories.
for source in /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
    if test -e "$source"; then
        echo 'Unexpected additional apt source; refusing snapshot repair' >&2
        exit 1
    fi
done
test -f /etc/apt/sources.list
mkdir -p /logs/verifier
cp /etc/apt/sources.list /logs/verifier/qemu-original-sources.list
cat > /etc/apt/sources.list <<'APT_SOURCES'
deb [check-valid-until=no signed-by=/usr/share/keyrings/debian-archive-keyring.gpg] https://snapshot.debian.org/archive/debian/20260824T000000Z bullseye main
deb [check-valid-until=no signed-by=/usr/share/keyrings/debian-archive-keyring.gpg] https://snapshot.debian.org/archive/debian-security/20260824T000000Z bullseye-security main
APT_SOURCES
# Old fixed snapshots need the scoped expiry exception above; archive signatures
# and package hashes remain mandatory. Fail before starting QEMU on index errors.
apt-get -o APT::Update::Error-Mode=any update
