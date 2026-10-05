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


def stage_downloaded(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id, ownership=None):
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
            if ownership is not None:
                stage_inventory(final, ownership)
            private_directory(final)
            owned_candidate(final / "rootfs.ext4")
            # A completed handoff has no source name. The same immutable
            # manifest/status/kernel must still match on every retry.
            return stage(root, kernel, final / "rootfs.ext4", status, owner, cpu,
                         memory_mib, host_memory_mib, candidate_id=candidate_id, ownership=ownership)
        if pending.exists():
            if ownership is not None:
                # Resume in the nonce-bound directory without deleting any
                # inode. An incomplete/unrecognized file fails closed and is
                # handled only by the supervisor's journaled cleanup protocol.
                stage_inventory(pending, ownership)
                owned_candidate(pending / "rootfs.ext4")
                return _stage(root, kernel, pending / "rootfs.ext4", status, owner, cpu,
                              memory_mib, host_memory_mib, candidate_id=candidate_id,
                              move_disk=False, ownership=ownership, resume_download=True)
            private_directory(pending)
            owned_candidate(pending / "rootfs.ext4")
            os.rename(pending / "rootfs.ext4", disk)
            sync_directory(disk.parent)
            sync_directory(pending)
    owned_candidate(disk)
    return _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib,
                  candidate_id=candidate_id, move_disk=True, ownership=ownership, resume_download=True)


def stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id=None, ownership=None):
    return _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib,
                  candidate_id=candidate_id, move_disk=False, ownership=ownership)


def stage_sync(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def stage_owner(path, ownership):
    """Recover only the journal-reserved unpublished marker staging cut."""
    private_directory(path)
    encoded = json.dumps(ownership, sort_keys=True).encode()
    marker, partial = path / ".build-owner.json", path / ".build-owner.partial"
    if marker.exists():
        return
    with os.scandir(path) as entries:
        names = []
        for entry in entries:
            if entry.name != ".build-owner.partial" or names:
                raise ValueError("unowned staging content")
            meta = entry.stat(follow_symlinks=False)
            if (not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_nlink != 1
                    or meta.st_size > len(encoded) or meta.st_mode & 0o077
                    or not encoded.startswith(Path(entry.path).read_bytes())):
                raise ValueError("unowned staging marker")
            names.append(entry.name)
    with partial.open("wb") as output:
        os.chmod(output.name, 0o600)
        output.write(encoded)
        output.flush()
        os.fsync(output.fileno())
    os.replace(partial, marker)
    stage_sync(path)
    stage_sync(path.parent)


def stage_inventory(path, ownership, *, initialize=False):
    """Exact bounded staging inventory; never recurse through guest output."""
    private_directory(path)
    if initialize:
        stage_owner(path, ownership)
    marker = path / ".build-owner.json"
    encoded = json.dumps(ownership, sort_keys=True).encode()
    if marker.is_symlink() or marker.stat().st_size != len(encoded) or marker.read_bytes() != encoded:
        raise ValueError("unknown staging ownership")
    files = []
    with os.scandir(path) as entries:
        for entry in entries:
            if entry.name not in {"kernel", "rootfs.ext4", "manifest.json", ".build-owner.json"} or len(files) >= 4:
                raise ValueError("unknown staging entry")
            meta = entry.stat(follow_symlinks=False)
            if not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_uid != os.geteuid():
                raise ValueError("unknown staging inode")
            files.append([entry.name, meta.st_dev, meta.st_ino])
    meta = path.stat()
    return {"directory": [meta.st_dev, meta.st_ino], "files": files}


def stage_delete(path, manifest):
    """Idempotent unlink of a previously committed exact manifest only."""
    if path.exists() or path.is_symlink():
        private_directory(path)
        meta = path.stat()
        if manifest["directory"] != [meta.st_dev, meta.st_ino]:
            raise ValueError("staging directory identity changed")
        expected = {name: (dev, ino) for name, dev, ino in manifest["files"]}
        files = []
        with os.scandir(path) as entries:
            for entry in entries:
                meta = entry.stat(follow_symlinks=False)
                if len(files) >= len(expected) or expected.get(entry.name) != (meta.st_dev, meta.st_ino):
                    raise ValueError("staging entry identity changed")
                files.append(entry)
        for entry in files:
            Path(entry.path).unlink()
        stage_sync(path)
        path.rmdir()
    stage_sync(path.parent)


def _stage(root, kernel, disk, status, owner, cpu, memory_mib, host_memory_mib, *, candidate_id,
           move_disk, ownership=None, resume_download=False):
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
    if final.exists() or final.is_symlink():
        if ownership is not None:
            stage_inventory(final, ownership)
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
        if pending.exists() or pending.is_symlink():
            private_directory(pending)
            if ownership is not None:
                # The journal must commit an exact deletion manifest first.
                # Never adopt or broadly remove interrupted/unrecognized data.
                stage_inventory(pending, ownership, initialize=True)
                if not resume_download:
                    raise ValueError("interrupted bounded staging requires journaled cleanup")
            else:
                shutil.rmtree(pending)  # legacy standalone operator mode
        if not pending.exists():
            pending.mkdir(mode=0o700)
    else:
        pending = Path(tempfile.mkdtemp(prefix=".stage-", dir=root / "inbox"))
    moved = False
    try:
        if ownership is not None:
            stage_owner(pending, ownership)
        artifacts = {}
        for name, source in (("kernel", kernel), ("rootfs.ext4", disk)):
            target = pending / name
            if ownership is not None and resume_download and target.exists():
                # A crash may leave a complete copied kernel or transferred
                # disk. Never overwrite/recopy it or adopt mismatching bytes.
                owned_candidate(target)
                expected_hash = rootfs_digest if name == "rootfs.ext4" else file_hash(source)
                if target.stat().st_size != source.stat().st_size or file_hash(target) != expected_hash:
                    raise ValueError("interrupted staging artifact mismatch")
            elif move_disk and name == "rootfs.ext4":
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
        manifest_path = pending / "manifest.json"
        if ownership is not None and resume_download and manifest_path.exists():
            if manifest_path.read_bytes() != encoded:
                raise ValueError("interrupted staging manifest mismatch")
            with manifest_path.open("rb") as output:
                os.fsync(output.fileno())
        else:
            with manifest_path.open("xb") as output:
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
        if pending.exists() and ownership is None:
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
