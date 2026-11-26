#!/usr/bin/env python3
"""Build the protected hot-fanout fixture from an approved fixed Ubuntu base."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import zipfile

if __package__:
    from .build_base_guest import PROFILE, debugfs, digest, safe_path, verified_copy, verify_image
    from .hot_fanout.control import identifier
else:
    from build_base_guest import PROFILE, debugfs, digest, safe_path, verified_copy, verify_image
    from hot_fanout.control import identifier

PROJECT = Path(__file__).resolve().parents[1]
PLATFORM = "/opt/sandproof-hot-fanout"
UNIT = "/etc/systemd/system/sandproof-hot-fanout.service"
LINK = "/etc/systemd/system/multi-user.target.wants/sandproof-hot-fanout.service"
MIB = 1024 * 1024
PARTIAL_START = "partial-start"
PARTIAL_START_EVIDENCE = "/run/sandproof-hot-fanout/partial-start.json"

SERVICE = f"""[Unit]
Description=Sandproof approved hot fan-out checkpoint
After=local-fs.target
[Service]
Type=simple
User=root
ExecStart={PLATFORM}/start
Restart=no
[Install]
WantedBy=multi-user.target
"""

START = f"""#!/bin/sh
set -eu
mkdir -m 0700 /run/sandproof-hot-lower
mount -t ext4 -o loop,ro,noload,nodev,nosuid {PLATFORM}/lower.ext4 /run/sandproof-hot-lower
exec /usr/bin/python3 -I -B {PLATFORM}/checkpoint.pyz
"""

WORKLOAD = """import hashlib
import mmap
import os
from pathlib import Path
from tools.hot_fanout.prepared import serve_prepared_checkpoint

page = os.sysconf("SC_PAGE_SIZE")
heap = mmap.mmap(-1, 64 * 1024**2, flags=mmap.MAP_PRIVATE | mmap.MAP_ANONYMOUS)
original = hashlib.shake_256(("sandproof-fanout-fixture:" + CHECKPOINT).encode()).digest(len(heap) // page)
heap[::page] = original

def on_branch(_):
    if heap[::page] != original:
        raise RuntimeError("forked fixture did not inherit the prepared memory")
    Path("/run/inherited").write_text(original[:16].hex())

serve_prepared_checkpoint(CHECKPOINT, ["/run/sandproof-hot-lower"],
                          "/usr/local/bin/envd", max_branches=BRANCHES,
                          max_batches=BATCHES, workload=on_branch)
"""

# This prefix is present only in the separately built fault image. The donor
# runs it once in its own process and uses the production start for all peers.
PARTIAL_START_WORKLOAD = """import errno
import json
from pathlib import Path
from tools.hot_fanout import linux

spawn_member = linux._spawn_member
previous_scopes = []
injected = False

def fail_third_once(cgroup, child):
    global injected
    if not injected:
        batch, ordinal = cgroup.name.rsplit(".", 1)
        if ordinal == "0":
            previous_scopes.clear()
        if ordinal == "2":
            injected = True
            complete_prefix = [scope.name for scope in previous_scopes] == [batch + ".0", batch + ".1"]
            if not complete_prefix or not all((scope / "cgroup.procs").read_text().strip()
                                              for scope in previous_scopes):
                raise OSError(errno.EIO, "fixture lacks the two original live child scopes")
            with Path(PARTIAL_START_EVIDENCE).open("x") as evidence:
                json.dump({"live_ordinals_before_failure": [0, 1], "failed_ordinal": 2}, evidence)
            raise OSError(errno.EAGAIN, "fixture stopped the third branch spawn")
    spawn_member(cgroup, child)
    if not injected:
        previous_scopes.append(cgroup)

linux._spawn_member = fail_third_once
"""


def expected_memory_mark(checkpoint):
    return hashlib.shake_256(("sandproof-fanout-fixture:" + checkpoint).encode()).hexdigest(16)


def install_archive(path, checkpoint, branch_cap, batch_cap, *, fault_case=None):
    prefix = f"CHECKPOINT = {checkpoint!r}\nBRANCHES = {branch_cap}\nBATCHES = {batch_cap}\n"
    if fault_case == PARTIAL_START:
        prefix += f"PARTIAL_START_EVIDENCE = {PARTIAL_START_EVIDENCE!r}\n" + PARTIAL_START_WORKLOAD
    with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("__main__.py", prefix + WORKLOAD)
        archive.write(PROJECT / "tools/__init__.py", "tools/__init__.py")
        for filename in ("__init__.py", "control.py", "linux.py", "service.py", "prepared.py", "DELTABOX-LICENSE"):
            archive.write(PROJECT / "tools/hot_fanout" / filename, f"tools/hot_fanout/{filename}")


def command(*args, timeout=180):
    subprocess.run(args, check=True, timeout=timeout, stdout=sys.stderr)


def put_file(disk, source, destination, mode, *, immutable=False):
    debugfs(disk, f"write {source} {destination}", writable=True)
    fields = {"mode": f"0{0o100000 | mode:o}", "uid": 0, "gid": 0}
    if immutable:
        stored = debugfs(disk, f"stat {destination}")
        found = re.search(r"Flags:\s+(0x[0-9a-f]+)", stored)
        if found is None:
            raise ValueError("could not observe staged file flags")
        fields["flags"] = f"{int(found[1], 16) | 0x10:#x}"
    for key, value in fields.items():
        debugfs(disk, f"set_inode_field {destination} {key} {value}", writable=True)
    stored = debugfs(disk, f"stat {destination}")
    if "Type: regular" not in stored or f"Mode:  {mode:04o}" not in stored:
        raise ValueError("staged fixture file type or mode differs")
    if immutable:
        found = re.search(r"Flags:\s+(0x[0-9a-f]+)", stored)
        if found is None or not int(found[1], 16) & 0x10:
            raise ValueError("staged fixture lower image is mutable")


def artifact(path):
    return {"path": str(path), "sha256": digest(path), "size_bytes": path.stat().st_size}


def build(args):
    if not sys.platform.startswith("linux"):
        raise ValueError("fixture image construction requires Linux/e2fsprogs")
    profile = json.loads(PROFILE.read_bytes())
    if not re.fullmatch(r"[0-9a-f]{64}", args.base_sha256):
        raise ValueError("use the approved official base build SHA-256")
    if (not 1 <= args.max_branches <= 64 or not 1 <= args.max_batches <= 1024
            or not 1 <= args.vcpus <= 32 or not 128 <= args.memory_mib < args.host_memory_mib
            or not 0 < args.owner <= (1 << 64) - 1):
        raise ValueError("invalid fixture resource bounds")
    if args.fault_case == PARTIAL_START and (args.max_branches != 4 or args.max_batches < 3):
        raise ValueError("partial-start fixture requires four branch slots and three batch records")
    for label in (args.template_id, args.revision, args.checkpoint):
        identifier(label)

    output = safe_path(args.output.resolve())
    if output.is_relative_to(PROJECT):
        raise ValueError("fixture volumes belong outside the source checkout")
    output.mkdir(mode=0o700)
    with tempfile.TemporaryDirectory(dir=output, prefix="prepare-") as temporary:
        work = safe_path(temporary)
        lower = work / "lower.ext4"
        verified_copy(args.base, lower, {"sha256": args.base_sha256, "bytes": profile["image"]["bytes"]})
        command("/usr/sbin/e2fsck", "-fn", str(lower))
        verify_image(lower, work, profile)

        disk = work / "rootfs.ext4"
        command("/usr/bin/cp", "--sparse=always", "--reflink=auto", str(lower), str(disk))
        disk.chmod(0o600)
        with disk.open("ab") as stream:
            stream.truncate(2 * lower.stat().st_size + 64 * MIB)
        command("/usr/sbin/resize2fs", str(disk))
        if "Inode:" not in debugfs(disk, "stat /opt"):
            debugfs(disk, "mkdir /opt", writable=True)
        debugfs(disk, f"mkdir {PLATFORM}", writable=True)
        start, unit, payload = work / "start", work / "unit", work / "checkpoint.pyz"
        start.write_text(START)
        unit.write_text(SERVICE)
        install_archive(payload, args.checkpoint, args.max_branches, args.max_batches, fault_case=args.fault_case)
        for source, destination, mode in (
            (start, PLATFORM + "/start", 0o500),
            (unit, UNIT, 0o444),
            (payload, PLATFORM + "/checkpoint.pyz", 0o400),
        ):
            put_file(disk, source, destination, mode)
        put_file(disk, lower, PLATFORM + "/lower.ext4", 0o444, immutable=True)
        debugfs(disk, f"symlink {LINK} {UNIT}", writable=True)
        if f'Fast link dest: "{UNIT}"' not in debugfs(disk, f"stat {LINK}"):
            raise ValueError("approved hot-fanout guest unit was not enabled")
        for source, destination in ((unit, UNIT), (start, PLATFORM + "/start"),
                                    (payload, PLATFORM + "/checkpoint.pyz"),
                                    (lower, PLATFORM + "/lower.ext4")):
            check = work / "readback"
            debugfs(disk, f"dump {destination} {check}")
            if digest(source) != digest(check):
                raise ValueError("sealed guest bytes differ from the approved fixture")
            check.unlink()
        command("/usr/sbin/e2fsck", "-fn", str(disk))
        verified_copy(args.kernel, output / "vmlinux", profile["kernel"])
        disk.chmod(0o444)
        disk.rename(output / "rootfs.ext4")

    binding = {"template_id": args.template_id, "revision": args.revision, "owner": args.owner,
               "kernel": artifact(output / "vmlinux"), "rootfs": artifact(output / "rootfs.ext4"),
               "resources": {"cpu_count": args.vcpus, "memory_mib": args.memory_mib,
                             "disk_bytes": (output / "rootfs.ext4").stat().st_size,
                             "host_memory_bytes": args.host_memory_mib * MIB},
               "envd_version": "0.9.0"}
    # TemplateBinding::canonical_digest serializes this exact struct order.
    # Cache projection to a different owner or paths yields a different local pin.
    pin = list(hashlib.sha256(json.dumps(binding, separators=(",", ":")).encode()).digest())
    ready = {"binding": binding, "lower_sha256": args.base_sha256,
             "envd_sha256": profile["envd"]["sha256"],
             "expected_inherited_mark": expected_memory_mark(args.checkpoint),
             "native_hot_fanout": {"template": {"descriptor_digest": pin, "checkpoint_id": args.checkpoint,
                                                "max_branches": args.max_branches},
                                   "max_batches": args.max_batches, "max_branches_per_vm": args.max_branches}}
    if args.fault_case is not None:
        ready["fault_case"] = args.fault_case
    receipt = output / "receipt.json"
    with receipt.open("x") as result:
        json.dump(ready, result, indent=2)
        result.write("\n")
    receipt.chmod(0o400)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("base", "kernel", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--base-sha256", required=True)
    parser.add_argument("--owner", type=int, required=True)
    parser.add_argument("--template-id", required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--max-branches", type=int, required=True)
    parser.add_argument("--max-batches", type=int, required=True)
    parser.add_argument("--vcpus", type=int, required=True)
    parser.add_argument("--memory-mib", type=int, required=True)
    parser.add_argument("--host-memory-mib", type=int, required=True)
    parser.add_argument("--fault-case", choices=(PARTIAL_START,),
                        help="build a separate checkpoint fixture for one explicit failure")
    print(build(parser.parse_args()))


if __name__ == "__main__":
    main()
