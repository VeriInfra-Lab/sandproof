#!/usr/bin/env python3
"""Trusted operator staging; does not build, unpack, mount or publish a template.

Run on the isolated root Linux host after the build VM has been reclaimed.
Only a candidate ID is exposed to the authenticated publication API.
"""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import stat
import sys
import tempfile

ZERO_BLOCK = bytes(64 * 1024)


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


def write_sparse(output, block):
    """Write opaque bytes to a new private file, seeking over zero spans.

    No filesystem/image parsing. The caller must truncate at the final logical
    position before fsync: a trailing seek alone does not extend the file.
    """
    for offset in range(0, len(block), len(ZERO_BLOCK)):
        chunk = block[offset:offset + len(ZERO_BLOCK)]
        if chunk == ZERO_BLOCK[:len(chunk)]:
            output.seek(len(chunk), os.SEEK_CUR)
        else:
            output.write(chunk)


def copy_sparse(source, target):
    """Copy opaque bytes into a distinct inode, preserving holes when supported."""
    with source.open("rb", buffering=0) as incoming, target.open("xb") as output:
        size = os.fstat(incoming.fileno()).st_size
        try:
            if not all(hasattr(os, name) for name in ("SEEK_DATA", "SEEK_HOLE", "sendfile")):
                raise OSError(errno.ENOSYS, "extent copy unavailable")
            position = 0
            while position < size:
                try:
                    data = os.lseek(incoming.fileno(), position, os.SEEK_DATA)
                except OSError as error:
                    if error.errno == errno.ENXIO:
                        break  # The remainder is a hole.
                    raise
                end = min(os.lseek(incoming.fileno(), data, os.SEEK_HOLE), size)
                if not position <= data < end:
                    raise OSError(errno.EIO, "invalid source extent")
                incoming.seek(data)
                output.seek(data)
                while data < end:
                    copied = os.sendfile(output.fileno(), incoming.fileno(), None, min(end - data, 8 * 1024 * 1024))
                    if copied == 0:
                        raise OSError(errno.EOPNOTSUPP, "extent copy made no progress")
                    data += copied
                position = end
        except OSError as error:
            if error.errno not in (errno.ENOSYS, errno.EINVAL, errno.EXDEV, errno.EOPNOTSUPP):
                raise
            # Unsupported filesystems/cross-device copies may fail after a
            # partial extent. Restart this new private output, never the source.
            incoming.seek(0)
            output.seek(0)
            output.truncate()
            remaining = size
            while remaining:
                block = incoming.read(min(remaining, 1024 * 1024))
                if not block:
                    raise OSError(errno.EIO, "source truncated during copy")
                write_sparse(output, block)
                remaining -= len(block)
        if os.fstat(incoming.fileno()).st_size != size:
            raise OSError(errno.EIO, "source size changed during copy")
        output.truncate(size)


def sync_directory(path):
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def owned_candidate(path):
    meta = path.lstat()
    if (not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid()
            or meta.st_nlink != 1 or stat.S_IMODE(meta.st_mode) not in (0o600, 0o400, 0o444)):
        raise ValueError("private single-link downloaded candidate required")


def stage_downloaded(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id):
    """Consume one supervisor-owned download after its builder was reclaimed.

    Same-filesystem rename transfers the inode; it never shares a writable
    inode with the guest or a second artifact name. Cross-device staging still
    copies. An interrupted transfer can resume from this exact pending/final
    identity, with the ordinary full fsynced readback and publication checks.
    """
    root, disk = Path(root), Path(disk)
    if (not isinstance(candidate_id, str) or len(candidate_id) != 32
            or any(c not in "0123456789abcdef" for c in candidate_id)
            or disk.name != "candidate.ext4" or disk.parent.name != candidate_id):
        raise ValueError("fixed supervisor candidate slot required")
    for path in (root, root / "inbox", root / "artifacts", disk.parent):
        private_directory(path)
    final = root / "inbox" / candidate_id
    pending = root / "inbox" / (".stage-" + candidate_id)
    if not disk.exists() and not disk.is_symlink():
        if final.exists():
            private_directory(final)
            owned_candidate(final / "rootfs.ext4")
            # A completed handoff has no source name. The same immutable
            # manifest/status/kernel must still match on every retry.
            return stage(root, kernel, final / "rootfs.ext4", status, owner, cpu,
                         memory_mib, host_memory_mib, candidate_id=candidate_id)
        if pending.exists():
            private_directory(pending)
            owned_candidate(pending / "rootfs.ext4")
            os.rename(pending / "rootfs.ext4", disk)
            sync_directory(disk.parent)
            sync_directory(pending)
    owned_candidate(disk)
    return _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib,
                  candidate_id=candidate_id, move_disk=True)


def stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id=None):
    return _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib,
                  candidate_id=candidate_id, move_disk=False)


def _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id, move_disk):
    root, kernel, disk = map(Path, (root, kernel, disk))
    for path in (root, root / "inbox", root / "artifacts"):
        private_directory(path)
    if (type(owner) is not int or not 0 < owner < 2**64 or type(cpu) is not int or not 1 <= cpu <= 255
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
    rootfs_digest = status.get("rootfs_sha256")
    if (disk.stat().st_size != status["disk_bytes"] or not isinstance(rootfs_digest, str)
            or len(rootfs_digest) != 64 or any(c not in "0123456789abcdef" for c in rootfs_digest)):
        raise ValueError("candidate disk mismatch")
    candidate = candidate_id or secrets.token_hex(16)
    if not isinstance(candidate, str) or len(candidate) != 32 or any(c not in "0123456789abcdef" for c in candidate):
        raise ValueError("invalid candidate identity")
    final = root / "inbox" / candidate
    if final.exists():
        if file_hash(disk) != rootfs_digest:
            raise ValueError("candidate disk mismatch")
        private_directory(final)
        manifest = json.loads((final / "manifest.json").read_text())
        binding = manifest["binding"]
        if (binding["owner"] != owner or binding["template_id"] != "tpl-" + candidate
                or binding["resources"] != {"cpu_count": cpu, "memory_mib": memory_mib,
                    "disk_bytes": status["disk_bytes"], "host_memory_bytes": host_memory_mib * 1024 * 1024}
                or binding["startup"] != status["runtime_defaults"] or manifest["image_digest"] != digest
                or manifest["min_free_bytes"] != status["min_free_bytes"]
                or file_hash(final / "rootfs.ext4") != status["rootfs_sha256"]
                or file_hash(final / "kernel") != file_hash(kernel)):
            raise ValueError("staged identity conflict")
        return candidate
    if candidate_id:
        pending = root / "inbox" / (".stage-" + candidate)
        if pending.exists():
            private_directory(pending)
            shutil.rmtree(pending)  # interrupted copy, exact stable build ID
        pending.mkdir(mode=0o700)
    else:
        pending = Path(tempfile.mkdtemp(prefix=".stage-", dir=root / "inbox"))
    moved = False
    try:
        artifacts = {}
        for name, source in (("kernel", kernel), ("rootfs.ext4", disk)):
            target = pending / name
            if move_disk and name == "rootfs.ext4":
                owned_candidate(source)
                try:
                    os.rename(source, target)
                    moved = True
                except OSError as error:
                    if error.errno != errno.EXDEV:
                        raise
                    copy_sparse(source, target)
            else:
                copy_sparse(source, target)
            target.chmod(0o444)
            with target.open("rb") as saved:
                os.fsync(saved.fileno())
            artifacts[name] = {"path": str(final / name), "size_bytes": target.stat().st_size, "sha256": file_hash(target)}
        if moved:
            sync_directory(disk.parent)
        # Validate the fsynced copy itself before exposing its manifest. A new
        # staging does not need another full source scan before this readback.
        if (artifacts["rootfs.ext4"]["size_bytes"] != status["disk_bytes"]
                or artifacts["rootfs.ext4"]["sha256"] != rootfs_digest):
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
        if moved and pending.exists():
            # A normal I/O/check failure restores the supervisor slot before
            # discarding temporary metadata. SIGKILL leaves the exact pending
            # inode for stage_downloaded to recover; no rebuild is required.
            if disk.exists() or disk.is_symlink():
                raise RuntimeError("download slot reappeared during handoff")
            os.rename(pending / "rootfs.ext4", disk)
            sync_directory(disk.parent)
            sync_directory(pending)
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
