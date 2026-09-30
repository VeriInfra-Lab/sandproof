#!/usr/bin/env python3
"""Trusted operator staging; does not build, unpack, mount or publish a template.

Run on the isolated root Linux host after the build VM has been reclaimed.
Only a candidate ID is exposed to the authenticated publication API.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import stat
import sys
import tempfile


def private_directory(path):
    path = Path(path)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError("absolute nonsymlink directory required")
    meta = path.stat()
    if not stat.S_ISDIR(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
        raise ValueError("private operator directory required")


def file_hash(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as incoming:
        while block := incoming.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib):
    root, kernel, disk = map(Path, (root, kernel, disk))
    for path in (root, root / "inbox", root / "artifacts"):
        private_directory(path)
    if (type(owner) is not int or not 0 < owner < 2**63 or type(cpu) is not int or not 1 <= cpu <= 255
            or type(memory_mib) is not int or not 128 <= memory_mib <= 2**32 - 1
            or type(host_memory_mib) is not int or not memory_mib < host_memory_mib <= 2**32 - 1):
        raise ValueError("invalid owner/resource shape")
    if (status.get("state") != "candidate" or status.get("ready") is not False
            or status.get("startup_applied") is not False or not isinstance(status.get("runtime_defaults"), dict)
            or type(status.get("disk_bytes")) is not int or status["disk_bytes"] <= 0
            or type(status.get("min_free_bytes")) is not int or not 0 <= status["min_free_bytes"] <= status["disk_bytes"]):
        raise ValueError("not a complete unpublished candidate")
    digest = status.get("image_digest", "")
    if (not isinstance(digest, str) or not digest.startswith("sha256:") or len(digest) != 71
            or any(c not in "0123456789abcdef" for c in digest[7:])):
        raise ValueError("invalid image digest")
    for path in (kernel, disk):
        meta = path.lstat()
        if not stat.S_ISREG(meta.st_mode) or meta.st_size <= 0:
            raise ValueError("regular artifact required")
    if disk.stat().st_size != status["disk_bytes"] or file_hash(disk) != status.get("rootfs_sha256"):
        raise ValueError("candidate disk mismatch")
    candidate = secrets.token_hex(16)
    final = root / "inbox" / candidate
    pending = Path(tempfile.mkdtemp(prefix=".stage-", dir=root / "inbox"))
    try:
        artifacts = {}
        for name, source in (("kernel", kernel), ("rootfs.ext4", disk)):
            target = pending / name
            shutil.copyfile(source, target)
            target.chmod(0o444)
            with target.open("rb") as saved:
                os.fsync(saved.fileno())
            artifacts[name] = {"path": str(final / name), "size_bytes": target.stat().st_size, "sha256": file_hash(target)}
        if artifacts["rootfs.ext4"]["sha256"] != status["rootfs_sha256"]:
            raise ValueError("candidate changed during staging")
        manifest = {"binding": {"template_id": "tpl-" + candidate, "revision": "r1", "owner": owner,
                    "kernel": artifacts["kernel"], "rootfs": artifacts["rootfs.ext4"],
                    "resources": {"cpu_count": cpu, "memory_mib": memory_mib, "disk_bytes": status["disk_bytes"],
                                  "host_memory_bytes": host_memory_mib * 1024 * 1024},
                    "envd_version": "0.9.0", "startup": status["runtime_defaults"]},
                    "image_digest": digest, "min_free_bytes": status["min_free_bytes"]}
        encoded = json.dumps(manifest).encode()
        if len(encoded) > 64 * 1024:
            raise ValueError("manifest too large")
        with (pending / "manifest.json").open("xb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        (pending / "manifest.json").chmod(0o444)
        descriptor = os.open(pending, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        os.rename(pending, final)
        descriptor = os.open(final.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        return candidate
    except BaseException:
        # Only this newly allocated staging directory; never a caller-selected target.
        if pending.exists():
            shutil.rmtree(pending)
        raise


if __name__ == "__main__":
    if sys.platform != "linux" or os.geteuid() != 0:
        raise SystemExit("root isolated Linux operator required")
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("root", "kernel", "disk", "status"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("owner", "cpu", "memory-mib", "host-memory-mib"):
        parser.add_argument("--" + name, type=int, required=True)
    args = parser.parse_args()
    if args.status.stat().st_size > 128 * 1024:
        raise SystemExit("status too large")
    print(stage(args.root, args.kernel, args.disk, json.loads(args.status.read_text()),
                args.owner, args.cpu, args.memory_mib, args.host_memory_mib))
