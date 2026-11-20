"""Retire private test template data after the caller confirms VM cleanup.

No shared templates, manifests, SQL, snapshot volumes or unknown runs are
selected. This helper does not establish that a VM stopped.
"""
import os
from pathlib import Path
import re
import stat

from tools.ci_retention import fsync_directory, reject_mounts
from tools.system_acceptance import digest, private_path, write_once


def cache_identity(path):
    path=private_path(Path(path),directory=True)
    info=path.stat()
    return dict(path=str(path),identity=[info.st_dev,info.st_ino])


def retire(caches, evidence):
    evidence=Path(evidence)
    rows=[]
    for item in caches:
        root=private_path(Path(item['path']),directory=True)
        if cache_identity(root)!=item:
            raise ValueError('original private template cache identity changed')
        if evidence.resolve().is_relative_to(root):
            raise ValueError('retirement evidence must remain outside cache')
        reject_mounts(root)
        if any((root/'inbox').iterdir()):
            raise ValueError('unfinished template download retained')
        artifacts=private_path(root/'artifacts',directory=True)
        for entry in artifacts.iterdir():
            private_path(entry,directory=True)
            if not re.fullmatch('[0-9a-f]{64}',entry.name):
                raise ValueError('unknown template entry retained')
            manifest=private_path(entry/'manifest.json')
            for name in ('rootfs.ext4','kernel'):
                path=entry/name
                info=path.lstat()
                if (not stat.S_ISREG(info.st_mode) or info.st_uid!=os.geteuid()
                        or info.st_nlink!=1 or info.st_mode & 0o022):
                    raise ValueError('only original private template data may be retired')
                rows.append(dict(path=str(path),identity=[info.st_dev,info.st_ino],
                    bytes=info.st_size,allocated_bytes=info.st_blocks*512,
                    manifest=str(manifest),manifest_sha256=digest(manifest)))
    # All validation and the independent durable intent precede any unlink.
    write_once(evidence,dict(kind='private-template-retirement-v1',files=rows,
        caller_confirmed_vm_cleanup=True,shared_templates_selected=False))
    for row in rows:
        path=Path(row['path']);info=path.lstat()
        if [info.st_dev,info.st_ino]!=row['identity'] or not stat.S_ISREG(info.st_mode):
            raise ValueError('private template file changed before retirement')
        path.unlink()
    for directory in {Path(row['path']).parent for row in rows}:
        fsync_directory(directory)
    write_once(evidence.with_name(evidence.stem+'-complete.json'),
        dict(removed_files=len(rows),allocated_bytes=sum(r['allocated_bytes'] for r in rows),
             intent_sha256=digest(evidence)))
