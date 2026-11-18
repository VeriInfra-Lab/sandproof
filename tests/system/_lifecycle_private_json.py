"""Bounded private JSON reads shared by lifecycle admission and readback."""
import hashlib
import json
import os
from pathlib import Path
import stat

from tools.check_kvm_host import unique_keys


def read_private_json(path, limit, *, mode=None, directory_fd=None):
    """Read bounded bytes and identity through one root-private descriptor."""
    path=Path(path)
    identity=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_mode,m.st_uid,m.st_nlink)
    name=str(path) if directory_fd is None else path.name
    metadata=lambda:os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
    before=metadata()
    if (not stat.S_ISREG(before.st_mode) or before.st_uid!=0 or before.st_nlink!=1
            or before.st_mode&0o077 or not 0<before.st_size<=limit
            or (mode is not None and stat.S_IMODE(before.st_mode)!=mode)):
        raise ValueError('original bounded private JSON file required')
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,dir_fd=directory_fd)
    with os.fdopen(fd,'rb') as stream:
        if identity(os.fstat(stream.fileno()))!=identity(before):
            raise ValueError('JSON file changed before protected read')
        raw=stream.read(limit+1)
        if (len(raw)!=before.st_size or len(raw)>limit
                or identity(os.fstat(stream.fileno()))!=identity(before)
                or identity(metadata())!=identity(before)):
            raise ValueError('JSON file changed during protected read')
    return raw, identity(before)


def original_json(path, limit, *, expected_sha256=None):
    raw, _ = read_private_json(path, limit)
    if expected_sha256 is not None and hashlib.sha256(raw).hexdigest() != expected_sha256:
        raise ValueError('original JSON differs from independently approved fingerprint')
    return json.loads(raw, object_pairs_hook=unique_keys)
