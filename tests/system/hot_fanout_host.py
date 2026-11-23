#!/usr/bin/env python3
"""Boot one disposable offline Firecracker VM to run hot_fanout_guest.py."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PROFILE = ROOT / "tests/system/base-inputs.json"
SDK_COMMIT = "ccaf9fc0ffe6ac39c7ec786af7608ab1de19467b"
DELTA_BOX_KERNEL_SHA256 = "0ba5e9ee744bcb2bac3391e967cf36cfeb10fd3add067055321cb136cd5004c0"

INIT = """#!/bin/sh
set -eu
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
trap 'sync; sleep 2; reboot -f' EXIT
mount -t proc proc /proc
mount -t sysfs sys /sys
mountpoint -q /dev || mount -t devtmpfs devtmpfs /dev
mount -t cgroup2 cgroup2 /sys/fs/cgroup
mount -t tmpfs -o mode=0755 tmpfs /run
mount -t tmpfs -o mode=1777 tmpfs /tmp
ip link set lo up
mount -o remount,rw /
mkdir /opt/sandproof-fanout-test
tar xf /sandproof-fanout.tar -C /opt/sandproof-fanout-test
cd /opt/sandproof-fanout-test
export PYTHONPATH=/opt/sandproof-fanout-test:/opt/sandproof-fanout-test/deps
/usr/bin/python3 -u hot_fanout_guest.py --envd /opt/sandproof-fanout-test/envd --branches @N@
echo SP_FANOUT_FINISHED
"""


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args):
    if os.geteuid() == 0 or not os.access("/dev/kvm", os.R_OK | os.W_OK):
        raise ValueError("run as the dedicated unprivileged Linux KVM user")
    profile = json.loads(PROFILE.read_bytes())
    kernel_hash = digest(args.kernel)
    expected_kernel = args.kernel_sha256 or profile["kernel"]["sha256"]
    if expected_kernel not in (profile["kernel"]["sha256"], DELTA_BOX_KERNEL_SHA256):
        raise ValueError("kernel is not the pinned official or DeltaBox test kernel")
    if not re.fullmatch("[0-9a-f]{64}", args.base_sha256):
        raise ValueError("use the fixed base SHA-256 from its build receipt")
    for actual, expected in ((digest(args.base), args.base_sha256),
                             (digest(args.envd), profile["envd"]["sha256"]),
                             (kernel_hash, expected_kernel)):
        if actual != expected:
            raise ValueError("fan-out input fingerprint mismatch")
    source = json.loads((args.sdk_site / "e2b-2.51.0.dist-info/direct_url.json").read_bytes())
    if source.get("vcs_info", {}).get("commit_id") != SDK_COMMIT:
        raise ValueError("fan-out test requires the pinned SDK build")
    firecracker = shutil.which(args.firecracker)
    if firecracker is None:
        raise ValueError("Firecracker executable is missing")
    firecracker = Path(firecracker).resolve()
    version = subprocess.check_output([firecracker, "--version"], text=True).splitlines()[0]
    if not version.endswith(" v1.16.1"):
        raise ValueError("fan-out fixture requires Firecracker v1.16.1")
    if not 2 <= args.branches <= 32:
        raise ValueError("branch fixture count must be between 2 and 32")

    output = args.output.resolve()
    output.mkdir(mode=0o700)
    with tempfile.TemporaryDirectory(dir=output, prefix="vm-") as directory:
        tmp = Path(directory)
        disk = tmp / "root.ext4"
        subprocess.run(["cp", "--sparse=always", "--reflink=auto", args.base, disk], check=True, timeout=60)
        disk.chmod(0o600)
        # Expand only the fixture copy for the installed SDK and upper layers.
        with disk.open("ab") as destination:
            destination.truncate(disk.stat().st_size + 512 * 1024 ** 2)
        subprocess.run(["resize2fs", disk], check=True, timeout=60, stdout=subprocess.DEVNULL)
        populate(tmp, disk, args)
        boot_args = "console=ttyS0 reboot=k panic=1 pci=off init=/sandproof-fanout-init"
        if kernel_hash == DELTA_BOX_KERNEL_SHA256:
            boot_args += " acpi=off"
        config = {
            "boot-source": {"kernel_image_path": str(args.kernel.resolve()), "boot_args": boot_args},
            "drives": [{"drive_id": "root", "path_on_host": str(disk),
                        "is_root_device": True, "is_read_only": False}],
            "machine-config": {"vcpu_count": 4, "mem_size_mib": 2048},
        }
        (tmp / "vm.json").write_text(json.dumps(config))
        serial = output / "serial.log"
        with serial.open("xb") as log:
            subprocess.run([firecracker, "--no-api", "--config-file", "vm.json"], cwd=tmp,
                           timeout=150, stdin=subprocess.DEVNULL, stdout=log,
                           stderr=subprocess.STDOUT, check=True)
        lines = serial.read_text(errors="replace").splitlines()
        reports = [json.loads(line.removeprefix("SP_FANOUT_RESULT ")) for line in lines
                   if line.startswith("SP_FANOUT_RESULT ")]
        if len(reports) != 1 or "SP_FANOUT_FINISHED" not in lines:
            raise RuntimeError("hot fan-out guest failed; inspect serial.log")
        report = reports[0]
        report.update(base_sha256=args.base_sha256, kernel_sha256=kernel_hash, sdk_commit=SDK_COMMIT,
                      envd_sha256=profile["envd"]["sha256"], firecracker=version, vcpus=4, memory_mib=2048)
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        return report


def populate(tmp, disk, args):
    def omit_cache(item):
        return None if "__pycache__" in Path(item.name).parts else item

    with tarfile.open(tmp / "payload.tar", "w", dereference=True) as payload:
        payload.add(args.sdk_site, arcname="deps", filter=omit_cache)
        payload.add(ROOT / "tools/__init__.py", arcname="tools/__init__.py")
        payload.add(ROOT / "tools/hot_fanout", arcname="tools/hot_fanout", filter=omit_cache)
        payload.add(ROOT / "tests/system/hot_fanout_guest.py", arcname="hot_fanout_guest.py")
        payload.add(ROOT / "tests/system/_assertions_required.py", arcname="_assertions_required.py")
        binary_info = payload.gettarinfo(str(args.envd), arcname="envd")
        binary_info.mode = 0o500
        with args.envd.open("rb") as binary:
            payload.addfile(binary_info, binary)
    (tmp / "init").write_text(INIT.replace("@N@", str(args.branches)))
    for command in ("write payload.tar /sandproof-fanout.tar", "write init /sandproof-fanout-init",
                    "set_inode_field /sandproof-fanout-init mode 0100755"):
        result = subprocess.run(["debugfs", "-w", "-R", command, disk], cwd=tmp,
                                capture_output=True, text=True, check=True, timeout=60)
        if any(not line.startswith("debugfs ") for line in result.stderr.splitlines()):
            raise RuntimeError(f"could not stage disposable fan-out image: {command}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("base", "kernel", "envd", "sdk-site", "output"):
        parser.add_argument("--" + name, required=True, type=lambda value: Path(value).resolve())
    parser.add_argument("--base-sha256", required=True)
    parser.add_argument("--kernel-sha256")
    parser.add_argument("--branches", type=int, default=4)
    parser.add_argument("--firecracker", default="firecracker")
    print(json.dumps(run(parser.parse_args())))


if __name__ == "__main__":
    main()
