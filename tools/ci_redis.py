#!/usr/bin/env python3
"""Fixed Redis CI material preparation. Never downloads or initializes a ledger.

Build the verified official archive as runner in a fresh directory; the trusted
preheat recipe quiesces runner processes before root seals only this material.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

PIN = {'version': '7.2.16', 'archive_sha256': '960a8ec15e34ff40e57ff16837b26b33bd81f2da6d24497bb63de532a323a18e'}
ARCHIVE_LIMIT = 8 * 1024 ** 2
BINARY_LIMIT = 64 * 1024 ** 2
INSTALLED = Path('/usr/local/lib/sandproof-ci/redis-7.2.16')


def regular_bytes(path, limit):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError('Redis material must be a bounded regular file')
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError('Redis material exceeds byte budget')
    return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def checked_archive(path):
    data = regular_bytes(path, ARCHIVE_LIMIT)
    if digest(data) != PIN['archive_sha256']:
        raise ValueError('Redis archive differs from fixed official input')
    return data


def build(archive, output):
    if os.geteuid() == 0:
        raise ValueError('Redis source compilation must be unprivileged')
    archive_bytes = checked_archive(archive)
    # The root preheat recipe creates this fresh private staging directory.
    if output.is_symlink() or not output.is_dir() or any(output.iterdir()):
        raise ValueError('Redis build output must be a fresh empty directory')
    with tempfile.TemporaryDirectory(prefix='sandproof-ci-redis-build-') as temporary:
        work = Path(temporary)
        with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode='r:gz') as source:
            members = source.getmembers()
            total = 0
            names = set()
            for member in members:
                name = PurePosixPath(member.name)
                total += member.size
                if (name.is_absolute() or '..' in name.parts or not name.parts
                        or name.parts[0] != 'redis-' + PIN['version']
                        or not (member.isfile() or member.isdir()) or member.name in names
                        or total > 128 * 1024 ** 2 or len(members) > 20_000):
                    raise ValueError('unexpected fixed Redis archive layout')
                names.add(member.name)
            source.extractall(work, members=members, filter='data')
        root = work / ('redis-' + PIN['version'])
        # Fixed build; no PR command, extra flags, system installation or tests.
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(work), 'LC_ALL': 'C'}
        subprocess.run(['/usr/bin/make', '-j2', 'MALLOC=libc', 'BUILD_TLS=no', 'redis-server'], cwd=root,
                       env=env, check=True, timeout=900)
        binary = root / 'src/redis-server'
        version = subprocess.run([str(binary), '--version'], capture_output=True, text=True,
                                 env=env, check=True, timeout=10).stdout
        if not re.fullmatch(r'Redis server v=' + re.escape(PIN['version']) + r' sha=[^\r\n]*\n?', version):
            raise ValueError('Redis build has wrong version')
        data = regular_bytes(binary, BINARY_LIMIT)
        (output / 'redis-server').write_bytes(data)
        (output / 'redis-server').chmod(0o555)
        receipt = dict(PIN, binary_sha256=digest(data), binary_bytes=len(data))
        (output / 'material.json').write_text(json.dumps(receipt, sort_keys=True) + '\n')
        return receipt


def check_material(directory, *, protected=False):
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('Redis material directory is missing or linked')
    receipt = json.loads(regular_bytes(directory / 'material.json', 4096))
    if (not isinstance(receipt, dict) or set(receipt) != {*PIN, 'binary_sha256', 'binary_bytes'}
            or any(receipt.get(k) != v for k, v in PIN.items())):
        raise ValueError('Redis material receipt has wrong input identity')
    data = regular_bytes(directory / 'redis-server', BINARY_LIMIT)
    if (type(receipt['binary_bytes']) is not int or receipt['binary_bytes'] != len(data)
            or receipt['binary_sha256'] != digest(data)
            or data[:6] != b'\x7fELF\x02\x01' or data[18:20] != b'\x3e\x00'):
        raise ValueError('Redis binary differs from sealed x86_64 material')
    if protected:
        # All parents and files must be root-owned and non-writable by runner.
        for path in [directory, *directory.parents, directory / 'material.json', directory / 'redis-server']:
            info = path.lstat()
            if info.st_uid != 0 or stat.S_ISLNK(info.st_mode) or info.st_mode & 0o022:
                raise ValueError('Redis material is not protected')
    return receipt


def seal(stage):
    if os.geteuid() != 0:
        raise ValueError('only controlled root preparation seals CI material')
    if stage.parent != Path('/var/tmp') or not stage.name.startswith('sandproof-ci-redis.'):
        raise ValueError('Redis stage cleanup target is not the recipe directory')
    receipt = check_material(stage)
    INSTALLED.parent.mkdir(parents=True, exist_ok=True)
    for parent in [INSTALLED.parent, *INSTALLED.parent.parents]:
        info = parent.lstat()
        if info.st_uid != 0 or stat.S_ISLNK(info.st_mode) or info.st_mode & 0o022:
            raise ValueError('Redis installation parent is not protected')
    if INSTALLED.exists() or INSTALLED.is_symlink():
        # Same fixed material may already be present in the derived golden disk.
        if check_material(INSTALLED, protected=True) != receipt:
            raise ValueError('existing Redis material differs; no overwrite')
    else:
        INSTALLED.mkdir(mode=0o755)
        for name in ('redis-server', 'material.json'):
            with (INSTALLED / name).open('xb') as destination:
                destination.write(regular_bytes(stage / name, BINARY_LIMIT))
            (INSTALLED / name).chmod(0o555 if name == 'redis-server' else 0o444)
        check_material(INSTALLED, protected=True)
    # Only a fresh stage made by the fixed root recipe can be reclaimed.
    shutil.rmtree(stage)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    builder = commands.add_parser('build')
    builder.add_argument('--archive', type=Path, required=True)
    builder.add_argument('--output', type=Path, required=True)
    sealer = commands.add_parser('seal')
    sealer.add_argument('--stage', type=Path, required=True)
    commands.add_parser('check')
    args = parser.parse_args(argv)
    try:
        if args.command == 'build':
            build(args.archive, args.output)
        elif args.command == 'seal':
            seal(args.stage)
        else:
            check_material(INSTALLED, protected=True)
            print(INSTALLED / 'redis-server')
        return 0
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print('CI Redis material refused: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
