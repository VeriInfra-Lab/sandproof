"""One component's deterministic archive of a clean committed candidate.

Only tar transport metadata is normalized. Source contents, tracked file kinds
and Git's executable bit are checked against the actual committed blobs.
"""
from pathlib import Path, PurePosixPath
import argparse
import hashlib
import io
import json
import os
import stat
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[3]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(root, *args, data=None):
    return subprocess.run(['git', *args], cwd=root, input=data,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout


def committed_inputs(root):
    root = root.resolve(strict=True)
    if git(root, 'status', '--porcelain', '-z', '--untracked-files=normal'):
        raise ValueError('archive requires the clean committed candidate; no unrelated WIP')
    commit = git(root, 'rev-parse', 'HEAD').decode().strip()
    tree = git(root, 'rev-parse', 'HEAD^{tree}').decode().strip()
    entries = []
    seen = set()
    for entry in git(root, 'ls-tree', '-rz', '--full-tree', 'HEAD').split(b'\0'):
        if not entry:
            continue
        header, name = entry.split(b'\t', 1)
        mode, kind, oid = header.decode().split()
        name = name.decode('utf-8')
        path = PurePosixPath(name)
        if (path.is_absolute() or '..' in path.parts or name != path.as_posix()
                or '\\' in name or name in seen or not path.parts):
            raise ValueError('unsafe or duplicate tracked archive path')
        if kind != 'blob' or mode not in ('100644', '100755'):
            raise ValueError('only regular tracked files are supported: ' + name)
        seen.add(name)
        entries.append((name, mode, oid))
    if not entries:
        raise ValueError('empty candidate tree')
    entries.sort()
    # Compare actual files with committed bytes, rather than trusting mtimes or
    # a clean diff alone. One batch avoids a subprocess for each source file.
    blobs = git(root, 'cat-file', '--batch', data=''.join(oid+'\n' for _, _, oid in entries).encode())
    cursor, files, contents = 0, {}, {}
    for name, mode, oid in entries:
        end = blobs.index(b'\n', cursor)
        actual_oid, kind, size = blobs[cursor:end].decode().split()
        if actual_oid != oid or kind != 'blob':
            raise ValueError('unexpected committed blob')
        begin, end = end + 1, end + 1 + int(size)
        data = blobs[begin:end]
        if len(data) != int(size) or blobs[end:end+1] != b'\n':
            raise ValueError('truncated committed blob')
        cursor = end + 1
        file = root / name
        for parent in file.parents:
            if parent == root:
                break
            if not stat.S_ISDIR(parent.lstat().st_mode):
                raise ValueError('non-directory or linked source ancestor: ' + name)
        info = file.lstat()
        if (not stat.S_ISREG(info.st_mode) or bool(info.st_mode & stat.S_IXUSR) != (mode == '100755')
                or file.read_bytes() != data):
            raise ValueError('actual candidate differs from committed file: ' + name)
        files[name] = dict(kind='regular', mode=mode, sha256=hashlib.sha256(data).hexdigest())
        contents[name] = data
    if cursor != len(blobs):
        raise ValueError('unexpected trailing committed blobs')
    if git(root, 'rev-parse', 'HEAD').decode().strip() != commit:
        raise ValueError('candidate commit changed during source read')
    return dict(schema=1, commit=commit, tree=tree, files=files), contents


def build(root, archive, manifest):
    before, contents = committed_inputs(root)
    if archive == manifest or archive.exists() or manifest.exists():
        raise ValueError('archive and manifest destinations must be new and distinct')
    with tarfile.open(archive, mode='x', format=tarfile.PAX_FORMAT) as tar:
        for name, data in contents.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if before['files'][name]['mode'] == '100755' else 0o644
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ''
            tar.addfile(info, io.BytesIO(data))
    after, _ = committed_inputs(root)
    if after != before:
        raise ValueError('candidate changed during archive creation')
    record = dict(before, archive_sha256=sha(archive))
    with manifest.open('x') as stream:
        stream.write(json.dumps(record, sort_keys=True, separators=(',', ':'))+'\n')
    return record


def verify(archive, manifest):
    """Verify an original historical archive without relabelling its commit."""
    if any(p.is_symlink() or not p.is_file() for p in (archive, manifest)):
        raise ValueError('source archive and manifest must be original regular files')
    record = json.loads(manifest.read_text())
    if record.get('schema') != 1 or sha(archive) != record['archive_sha256']:
        raise ValueError('original source archive differs from its complete manifest')
    actual = {}
    with tarfile.open(archive) as tar:
        for entry in tar.getmembers():
            name = entry.name
            path = PurePosixPath(name)
            if (path.is_absolute() or '..' in path.parts or name != path.as_posix()
                    or not path.parts or '\\' in name or name in actual or not entry.isfile()
                    or entry.mode not in (0o644, 0o755)):
                raise ValueError('invalid, duplicate or non-regular archived source')
            actual[name] = dict(kind='regular', mode='100755' if entry.mode == 0o755 else '100644',
                sha256=hashlib.sha256(tar.extractfile(entry).read()).hexdigest())
    if actual != record['files']:
        raise ValueError('original archive source list/content/type/execute bit differs')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = build(args.root, args.archive.absolute(), args.manifest.absolute())
    print(json.dumps(dict(commit=result['commit'], files=len(result['files']),
                          archive_sha256=result['archive_sha256'])))
