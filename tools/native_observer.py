"""Inspect Linux references to private native build paths without modifying them."""

import argparse
import json
import os
from pathlib import Path
import re


PROC = Path('/proc')
MOUNT_ESCAPE = re.compile(r'\\([0-7]{3})')


def covers(parent, path):
    return path == parent or path.startswith(parent.rstrip('/') + '/')


def mount_rows(text):
    for line in text.splitlines():
        fields = line.split()
        if len(fields) < 10 or '-' not in fields[6:]:
            raise ValueError('invalid mountinfo row')
        root, target = [MOUNT_ESCAPE.sub(lambda m: chr(int(m[1], 8)), raw)
                        for raw in fields[3:5]]
        yield fields[2], root, target


def accept_path(raw, runs_root, uid):
    path = Path(raw)
    if not path.is_absolute() or str(path) != raw or '..' in path.parts:
        raise ValueError('observer inputs must be normalized absolute paths')
    canonical = path.resolve()
    for parent in canonical.parents:
        if parent.exists() and parent.samefile(runs_root):
            job = canonical.relative_to(parent).parts[0]
            job_path = parent / job
            info = job_path.stat()
            if info.st_uid != uid or info.st_mode & 0o077:
                raise ValueError('observer requires an owner-private enrolled job')
            return canonical
    raise ValueError('observer path must be beneath the configured runs directory')


def scan(paths):
    hits = []
    errors = []
    local_mounts = list(mount_rows((PROC / 'self/mountinfo').read_text()))
    filesystems = []
    for path in paths:
        device, root, mount = max((row for row in local_mounts if covers(row[2], str(path))),
                                  key=lambda row: len(row[2]))
        filesystems.append((device, str(Path(root) / path.relative_to(mount))))

    def observed(path, read):
        try:
            return read(path)
        except (FileNotFoundError, ProcessLookupError):
            return None
        except OSError as error:
            errors.append({'path': str(path), 'errno': error.errno})
            return None

    def inspect(pid, role, raw, roots):
        if raw and any(covers(root, raw.removesuffix(' (deleted)')) for root in roots):
            hits.append({'pid': pid, 'role': role})

    pids = sorted(path for path in PROC.iterdir() if path.name.isdecimal())
    namespaces = {}
    for proc in pids:
        pid = int(proc.name)
        namespace = observed(proc / 'ns/mnt', os.readlink)
        if namespace is None:
            continue
        process_root = observed(proc / 'root', os.readlink)
        if process_root is None:
            continue
        namespace = namespace, process_root
        if namespace not in namespaces:
            mounts = observed(proc / 'mountinfo', Path.read_text)
            if mounts is None:
                continue
            roots = {str(path) for path in paths}
            rows = list(mount_rows(mounts))
            for device, root, target in rows:
                for source_device, source in filesystems:
                    if device != source_device:
                        continue
                    if covers(source, root):
                        hits.append({'pid': pid, 'role': 'mount'})
                    if covers(root, source):
                        roots.add(str(Path(target) / Path(source).relative_to(root)))
            if any(covers(root, target) for _, _, target in rows for root in roots):
                hits.append({'pid': pid, 'role': 'mountpoint'})
            namespaces[namespace] = roots
        roots = namespaces[namespace]
        inspect(pid, 'root', process_root, roots)
        for role in ('cwd', 'exe'):
            inspect(pid, role, observed(proc / role, os.readlink), roots)
        for fd in observed(proc / 'fd', lambda p: list(p.iterdir())) or ():
            inspect(pid, 'fd', observed(fd, os.readlink), roots)
        for row in (observed(proc / 'maps', Path.read_text) or '').splitlines():
            fields = row.split(None, 5)
            if len(fields) == 6:
                inspect(pid, 'mapped', fields[5], roots)

    return {'examined_processes': len(pids), 'matching_references': hits,
            'observation_errors': errors,
            'complete_no_references_or_mounts': bool(pids) and not hits and not errors}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runs-root', required=True, type=Path)
    parser.add_argument('paths', nargs='+')
    args = parser.parse_args()
    if os.geteuid() != 0 or not os.environ.get('SUDO_UID', '').isdigit():
        parser.error('the installed host launcher must provide privileged process visibility')
    runs_root = args.runs_root.resolve(strict=True)
    uid = int(os.environ['SUDO_UID'])
    if not runs_root.is_relative_to('/mnt') or runs_root.stat().st_uid != uid:
        parser.error('runs root must be the caller-owned directory on data storage')
    roots = {accept_path(path, runs_root, uid) for path in args.paths}
    report = scan(roots)
    print(json.dumps(report, sort_keys=True))
    return 0 if report['complete_no_references_or_mounts'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
