"""Operator-owned, read-only directory identity for the existing runtime.

This is execution provenance, not a cryptographic certificate from an untrusted
party. The host creates the lease; its existing wrapper mounts that exact file
read-only. No new tools or broad data-disk mounts are required.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path

SCHEMA = 'sandproof-bounded-writer-runtime-lease-v1'
PRIVATE_SCHEMA = 'sandproof-bounded-writer-runtime-lease-v2'
BWRAP_SHA256 = 'f99bf784b864f142683940b140b6444728b99accd1815768dc0b70298c40da38'
ROLES = ('source', 'work', 'target')


def absolute(path):
    path = Path(path)
    if not path.is_absolute() or str(path) != os.path.normpath(str(path)) or '..' in path.parts:
        raise ValueError('canonical absolute mapping path required')
    return path


def separate(paths):
    for index, first in enumerate(paths):
        for second in paths[index + 1:]:
            if first == second or first in second.parents or second in first.parents:
                raise ValueError('source/work/target must be separate non-nested directories')


def create(physical, runtime, physical_root=None):
    # The legacy data1 location remains admitted unchanged. A new host may
    # explicitly allocate an operator-owned private root instead.
    root = absolute(physical_root) if physical_root is not None else Path('/data1/zn')
    if physical_root is not None:
        if len(root.parts) < 3 or root.resolve(strict=True) != root or not root.is_dir():
            raise ValueError('real private physical root required')
        identity = root.stat()
        if identity.st_uid != os.getuid() or identity.st_mode & 0o077:
            raise ValueError('physical root must be owned by the operator and private')
    directories = {}
    for role in ROLES:
        path = absolute(physical[role])
        if path.resolve(strict=True) != path or not path.is_dir():
            raise ValueError('physical directory must be real, without symlink aliases')
        path.relative_to(root)
        if path == root:
            raise ValueError('allocate separate directories below the physical root')
        destination = absolute(runtime[role])
        identity = path.stat()
        directories[role] = dict(physical_path=str(path), runtime_path=str(destination),
                                 st_dev=identity.st_dev, st_ino=identity.st_ino)
    separate([Path(row['physical_path']) for row in directories.values()])
    separate([Path(row['runtime_path']) for row in directories.values()])
    document = dict(schema=SCHEMA, directories=directories, bwrap_sha256=BWRAP_SHA256)
    if physical_root is not None:
        document.update(schema=PRIVATE_SCHEMA, physical_root=str(root))
    return document


def validate(document):
    private = document.get('schema') == PRIVATE_SCHEMA
    fields = {'schema', 'directories', 'bwrap_sha256'} | ({'physical_root'} if private else set())
    if (set(document) != fields or document['schema'] not in (SCHEMA, PRIVATE_SCHEMA)
            or document['bwrap_sha256'] != BWRAP_SHA256 or set(document['directories']) != set(ROLES)):
        raise ValueError('unregistered runtime lease schema/tool')
    root = absolute(document['physical_root']) if private else Path('/data1/zn')
    if len(root.parts) < 3:
        raise ValueError('private physical root required, not a top-level directory')
    paths = {}
    physical = []
    for role in ROLES:
        row = document['directories'][role]
        if set(row) != {'physical_path', 'runtime_path', 'st_dev', 'st_ino'}:
            raise ValueError('incomplete directory identity')
        host = absolute(row['physical_path'])
        host.relative_to(root)
        if host == root:
            raise ValueError('whole physical-root lease is forbidden')
        path = absolute(row['runtime_path'])
        if path.resolve(strict=True) != path or not path.is_dir():
            raise ValueError('runtime mapping must be an exact real directory')
        if (type(row['st_dev']) is not int or type(row['st_ino']) is not int
                or row['st_dev'] <= 0 or row['st_ino'] <= 0):
            raise ValueError('invalid host directory identity')
        identity = path.stat()
        if (identity.st_dev, identity.st_ino) != (row['st_dev'], row['st_ino']):
            raise ValueError('runtime mapping differs from host directory: ' + role)
        paths[role] = path
        physical.append(host)
    separate(physical)
    separate(list(paths.values()))
    return paths


def admit(lease, executable):
    if lease.is_symlink() or not lease.is_file():
        raise ValueError('read-only operator lease file required')
    if not os.statvfs(lease).f_flag & os.ST_RDONLY:
        raise ValueError('operator lease must be mounted read-only, not merely chmodded')
    paths = validate(json.loads(lease.read_text()))
    executable = absolute(executable)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError('registered proof-bwrap alias required')
    if hashlib.sha256(executable.read_bytes()).hexdigest() != BWRAP_SHA256:
        raise ValueError('proof-bwrap binary differs from the measured existing tool')
    return paths


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    creator = sub.add_parser('create', help='host only, before read-only mounting')
    for role in ROLES:
        creator.add_argument('--' + role, type=Path, required=True)
        creator.add_argument('--' + role + '-runtime', type=Path, required=True)
    creator.add_argument('--physical-root', type=Path,
                         help='explicit private operator-owned root on a new host; omitted keeps the original data1 contract')
    creator.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    document = create({role: getattr(args, role) for role in ROLES},
                      {role: getattr(args, role + '_runtime') for role in ROLES},
                      physical_root=args.physical_root)
    with args.output.open('x') as stream:
        json.dump(document, stream, indent=2)
        stream.write('\n')
