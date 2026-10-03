#!/usr/bin/env bash
# Runs only while preparing the trusted base, never as a PR step.
set -euo pipefail
trap '/usr/sbin/poweroff' EXIT
set -a
source /etc/sandproof-ci.env
set +a
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  build-essential pkg-config libssl-dev git curl ca-certificates python3-venv \
  jq unzip libicu74 libkrb5-3 liblttng-ust1t64 libunwind8
install -d -o runner -g runner /opt/actions-runner
runner_url="$(python3 -c 'import json; print(json.load(open("/root/ci-pins.json"))["runner_url"])')"
runner_sha="$(python3 -c 'import json; print(json.load(open("/root/ci-pins.json"))["runner_sha256"])')"
rustup_sha="$(python3 -c 'import json; print(json.load(open("/root/ci-pins.json"))["rustup_sha256"])')"
rust_toolchain="$(python3 -c 'import json; print(json.load(open("/root/ci-pins.json"))["rust_toolchain"])')"
curl -fsSL --retry 3 "$runner_url" -o /tmp/actions-runner.tar.gz
printf '%s  %s\n' "$runner_sha" /tmp/actions-runner.tar.gz | sha256sum -c -
tar -xzf /tmp/actions-runner.tar.gz -C /opt/actions-runner --no-same-owner
chown -R runner:runner /opt/actions-runner
curl -fsSL --retry 3 https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init
printf '%s  %s\n' "$rustup_sha" /tmp/rustup-init | sha256sum -c -
chmod 0755 /tmp/rustup-init
/usr/sbin/runuser -u runner -- /tmp/rustup-init -y --profile minimal --default-toolchain "$rust_toolchain" --component clippy,rustfmt
/usr/sbin/runuser -u runner -- /home/runner/.cargo/bin/rustc --version
/opt/actions-runner/bin/Runner.Listener --version
systemctl disable --now ssh.service ssh.socket || true
apt-get clean
printf 'SANDPROOF_CI_IMAGE_READY\n'
