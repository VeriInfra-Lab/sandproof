"""Compare fixed umoci rootfs extraction inside the offline disposable VM."""

import sys as _guard_sys
if _guard_sys.flags.optimize:
    raise RuntimeError("optimized_python")


import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import stat
import sys
import tarfile
import time

sys.path.insert(0, "/input")
from template_inputs import inspect_oci, sha256
from template_worker import Commands, unpack_rootfs


def metadata(path):
    value = path.lstat()
    # Reads change atime; ctime/inode numbers identify different extractions.
    record = {"mode": value.st_mode, "uid": value.st_uid, "gid": value.st_gid,
              "mtime_ns": value.st_mtime_ns,
              "xattrs": {name: os.getxattr(path, name, follow_symlinks=False).hex()
                         for name in sorted(os.listxattr(path, follow_symlinks=False))}}
    if stat.S_ISREG(value.st_mode):
        record.update(size=value.st_size, sha256=sha256(path))
    elif stat.S_ISLNK(value.st_mode):
        record["target"] = os.readlink(path)
    else:
        assert stat.S_ISDIR(value.st_mode), path
    return record


def tree(root):
    records, inodes = {".": metadata(root)}, {}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            relative = str(path.relative_to(root))
            records[relative] = metadata(path)
            value = path.lstat()
            if stat.S_ISREG(value.st_mode):
                inodes.setdefault((value.st_dev, value.st_ino), []).append(relative)
    links = sorted(sorted(names) for names in inodes.values() if len(names) > 1)
    return records, links


def fixture(layout):
    blobs = layout / "blobs/sha256"
    blobs.mkdir(parents=True)
    def blob(data, media):
        digest = hashlib.sha256(data).hexdigest()
        (blobs / digest).write_bytes(data)
        return {"mediaType": "application/vnd.oci.image." + media,
                "digest": "sha256:" + digest, "size": len(data)}
    def archive(rows):
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w", format=tarfile.PAX_FORMAT) as tar:
            for name, kind, content in rows:
                info = tarfile.TarInfo(name)
                info.type, info.uid, info.gid, info.mtime = kind, 1234, 2345, 1234567890
                info.mode = 0o2750 if kind == tarfile.DIRTYPE else 0o640
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    info.linkname = content
                    info.mode = 0o777 if kind == tarfile.SYMTYPE else 0o4750
                elif kind == tarfile.REGTYPE:
                    info.size = len(content)
                if name == "owned/data":
                    info.mode = 0o4750
                    info.pax_headers = {"SCHILY.xattr.user.sandproof": "preserved-value"}
                tar.addfile(info, io.BytesIO(content) if kind == tarfile.REGTYPE else None)
        return output.getvalue()
    layers = [archive([
        ("owned", tarfile.DIRTYPE, b""),
        ("owned/data", tarfile.REGTYPE, b"metadata-and-bytes\n" * 262144),
        ("owned/hard", tarfile.LNKTYPE, "owned/data"),
        ("absolute", tarfile.SYMTYPE, "/owned/data"),
        ("relative", tarfile.SYMTYPE, "owned/data"),
        ("removed", tarfile.REGTYPE, b"must disappear"),
        ("opaque", tarfile.DIRTYPE, b""),
        ("opaque/old", tarfile.REGTYPE, b"must disappear")]), archive([
        (".wh.removed", tarfile.REGTYPE, b""),
        ("opaque/.wh..wh..opq", tarfile.REGTYPE, b""),
        ("opaque/kept", tarfile.REGTYPE, b"new layer")])]
    config = {"architecture": "amd64", "os": "linux", "config": {},
              "rootfs": {"type": "layers", "diff_ids": ["sha256:" + hashlib.sha256(x).hexdigest() for x in layers]}}
    manifest = {"schemaVersion": 2, "config": blob(json.dumps(config).encode(), "config.v1+json"),
                "layers": [blob(gzip.compress(x), "layer.v1.tar+gzip") for x in layers]}
    descriptor = blob(json.dumps(manifest).encode(), "manifest.v1+json")
    descriptor["annotations"] = {"org.opencontainers.image.ref.name": "candidate"}
    (layout / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [descriptor]}))
    (layout / "oci-layout").write_text(json.dumps({"imageLayoutVersion": "1.0.0"}))


def exercise():
    work = Path("/output/unpack-comparison")
    work.mkdir(mode=0o700)
    fixture(work / "metadata-oci")
    results = []
    with (work / "unpack.log").open("xb") as log:
        commands = Commands(time.monotonic() + 120, log, 2 * 1024 * 1024)
        for name, layout in (("built-image", Path("/output/job/oci")), ("metadata", work / "metadata-oci")):
            image = inspect_oci(layout, max_bytes=512 * 1024 * 1024, max_entries=10000)
            before = time.monotonic()
            commands.run(["/input/tools/umoci", "unpack", "--image", str(layout) + ":candidate", str(work / (name + "-bundle"))])
            bundle_seconds = time.monotonic() - before
            before = time.monotonic()
            raw = unpack_rootfs(commands, "/input/tools/umoci", layout, work / (name + "-raw"))
            raw_seconds = time.monotonic() - before
            expected = tree(work / (name + "-bundle/rootfs"))
            actual = tree(raw)
            assert actual == expected, name
            assert list(raw.parent.iterdir()) == [raw]
            assert stat.S_IMODE(raw.parent.stat().st_mode) == 0o700
            assert list((work / (name + "-bundle")).glob("*.mtree"))
            if name == "metadata":
                assert not (raw / "removed").exists() and not (raw / "opaque/old").exists()
                assert (raw / "opaque/kept").read_bytes() == b"new layer"
                assert actual[1] == [["owned/data", "owned/hard"]]
                assert (raw / "owned/data").stat().st_uid == 1234
                assert (raw / "owned/data").stat().st_gid == 2345
                assert stat.S_IMODE((raw / "owned/data").stat().st_mode) == 0o4750
                assert os.getxattr(raw / "owned/data", "user.sandproof") == b"preserved-value"
            results.append({"fixture": name, "image_digest": image["image_digest"],
                            "entries": len(actual[0]), "hardlink_groups": actual[1],
                            "bundle_seconds": bundle_seconds, "raw_seconds": raw_seconds})
    print(json.dumps({"same_bytes_owners_modes_mtimes_links_xattrs": True,
                      "same_whiteouts": True, "no_unused_bundle_history": True,
                      "timings_scope": "small synthetic fixtures, warm second extraction; not MTEB performance", "cases": results}))


if __name__ == "__main__":
    exercise()
