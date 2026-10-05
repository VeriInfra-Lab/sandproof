#!/bin/bash
# Guest-only setup for the pinned Ubuntu RStan-to-PyStan task.
set -eu
. /etc/os-release
test "$ID" = ubuntu
test "$VERSION_CODENAME" = noble
test -s /usr/share/keyrings/ubuntu-archive-keyring.gpg
for input in train_X.csv train_y.csv test_X.csv meta_public.json gp_rstan.R; do
    test -f "/app/$input"
done
test ! -e /etc/apt/sources.list.d/sandproof-pystan-snapshot.list
test ! -e /etc/apt/preferences.d/sandproof-pystan-curl
cat > /etc/apt/sources.list.d/sandproof-pystan-snapshot.list <<'APT_SOURCES'
deb [check-valid-until=no signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] https://snapshot.ubuntu.com/ubuntu/20251001T000000Z noble-updates main
APT_SOURCES
# Keep the original curl pin and its exact libcurl dependency consistent.
cat > /etc/apt/preferences.d/sandproof-pystan-curl <<'APT_PREFERENCES'
Package: curl libcurl4t64
Pin: version 8.5.0-2ubuntu10.6
Pin-Priority: 1001
APT_PREFERENCES
apt-get -o APT::Update::Error-Mode=any update
