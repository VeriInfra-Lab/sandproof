"""Read-only Rust guest observation through the existing authenticated SDK.

The program sent to the guest uses only Python's standard library. It observes
installed bytes and the process holding the configured listening socket; it
does not start a daemon, initialize state, repair, signal or install anything.
Guest root and the kernel are trusted for this engineering observation.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import sys

PLATFORM = '/opt/sandproof'
NAMES = {'service': 'sandproof-guest-service', 'worker': 'sandproof-guest-worker',
         'configuration': 'guest-config.json', 'manifest': 'guest-manifest.json'}
LIMITS = {'service': 256 * 1024**2, 'worker': 256 * 1024**2,
          'configuration': 1024**2, 'manifest': 16 * 1024}
PROGRAM_LIMIT = 24 * 1024
REPORT_LIMIT = 4096
PROCESS_LIMIT = 4096
FD_LIMIT = 4096


def require(condition):
    if not condition:
        raise ValueError('guest identity observation failed')


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def validate_expected(expected):
    require(type(expected) is dict and expected.keys() == NAMES.keys())
    for role, value in expected.items():
        require(type(value) is dict and value.keys() == {'bytes', 'sha256'})
        require(type(value['bytes']) is int and 0 < value['bytes'] <= LIMITS[role])
        require(type(value['sha256']) is str and re.fullmatch('[0-9a-f]{64}', value['sha256']))


def identity(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid,
            info.st_nlink, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def hashed(fd, expected, owner):
    before = os.fstat(fd)
    require(stat.S_ISREG(before.st_mode) and before.st_uid == owner
            and not before.st_mode & 0o022 and before.st_nlink == 1
            and before.st_size == expected['bytes'])
    total, digest = 0, hashlib.sha256()
    while True:
        block = os.read(fd, min(65536, expected['bytes'] - total + 1))
        if not block:
            break
        total += len(block)
        require(total <= expected['bytes'])
        digest.update(block)
    require(total == expected['bytes'] and digest.hexdigest() == expected['sha256']
            and identity(os.fstat(fd)) == identity(before))
    return before


def installed(platform, expected, owner):
    observed = {}
    for role, name in NAMES.items():
        path = platform / name
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            info = hashed(fd, expected[role], owner)
            require(identity(path.lstat()) == identity(info))
            require(stat.S_IMODE(info.st_mode) == (0o500 if role in ('service', 'worker') else 0o400))
            observed[role] = info
        finally:
            os.close(fd)
    return observed


def small_file(name, limit, *, directory=None):
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    try:
        chunks, total = [], 0
        while True:
            block = os.read(fd, min(4096, limit - total + 1))
            if not block:
                return b''.join(chunks)
            total += len(block)
            require(total <= limit)
            chunks.append(block)
    finally:
        os.close(fd)


def listener_inode(proc, port):
    lines = small_file(proc / 'net/tcp', 1024**2).splitlines()
    found = []
    for line in lines[1:]:
        fields = line.split()
        require(len(fields) >= 10)
        if fields[1] == f'00000000:{port:04X}'.encode() and fields[3] == b'0A':
            require(fields[9].isdigit() and int(fields[9]) > 0)
            found.append(int(fields[9]))
    require(len(found) == 1)
    return found[0]


def start_ticks(directory, pid):
    raw = small_file('stat', 4096, directory=directory)
    # comm may itself contain spaces and parentheses. The last ')' precedes
    # the fixed numeric fields; starttime is field 22, index 19 after comm.
    prefix = f'{pid} ('.encode()
    end = raw.rfind(b')')
    require(raw.startswith(prefix) and end >= len(prefix) and raw[end + 1:end + 2] == b' ')
    fields = raw[end + 1:].split()
    require(len(fields) > 19 and fields[19].isdigit() and int(fields[19]) > 0)
    return int(fields[19])


def owns_socket(directory, inode):
    fd = os.open('fd', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
    try:
        with os.scandir(fd) as entries:
            for count, entry in enumerate(entries, 1):
                require(count <= FD_LIMIT)
                try:
                    target = os.readlink(entry.name, dir_fd=fd)
                except FileNotFoundError:
                    continue
                if target == f'socket:[{inode}]':
                    return True
        return False
    finally:
        os.close(fd)


def observe_daemon(proc, platform, expected, installed_service, owner):
    inode = listener_inode(proc, 49983)
    argv = (str(platform / NAMES['service']) + '\0serve\0--config\0'
            + str(platform / NAMES['configuration']) + '\0').encode()
    matches = []
    with os.scandir(proc) as entries:
        count = 0
        for entry in entries:
            if not entry.name.isdecimal():
                continue
            count += 1
            require(count <= PROCESS_LIMIT)
            try:
                directory = os.open(entry.path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            except FileNotFoundError:
                continue
            try:
                try:
                    # Avoid reading other processes' arguments, including the
                    # observer's own larger Python source argument. Only an
                    # executable matching the installed service is a candidate.
                    if identity(os.stat('exe', dir_fd=directory)) != identity(installed_service):
                        continue
                    command = small_file('cmdline', 4096, directory=directory)
                except (FileNotFoundError, ProcessLookupError):
                    continue
                if command != argv:
                    continue
                pid = int(entry.name)
                before = start_ticks(directory, pid)
                if not owns_socket(directory, inode):
                    continue
                # Deliberately follow only the kernel's exe magic link below
                # the held proc directory, never an operator-supplied symlink.
                executable = os.open('exe', os.O_RDONLY | os.O_NONBLOCK, dir_fd=directory)
                try:
                    actual = hashed(executable, expected['service'], owner)
                    require(identity(actual) == identity(installed_service))
                    require(identity(os.stat('exe', dir_fd=directory)) == identity(actual))
                finally:
                    os.close(executable)
                require(start_ticks(directory, pid) == before
                        and small_file('cmdline', 4096, directory=directory) == argv
                        and owns_socket(directory, inode))
                matches.append({'pid': pid, 'start_ticks': before, 'socket_inode': inode})
            finally:
                os.close(directory)
    require(len(matches) == 1 and listener_inode(proc, 49983) == inode)
    return matches[0]


def observe(expected):
    validate_expected(expected)
    require(sys.platform == 'linux' and os.geteuid() == 0)
    platform = Path(PLATFORM)
    before = installed(platform, expected, 0)
    daemon = observe_daemon(Path('/proc'), platform, expected, before['service'], 0)
    after = installed(platform, expected, 0)
    require(all(identity(before[role]) == identity(after[role]) for role in NAMES))
    return {'schema_version': 1, 'files': expected, 'daemon': daemon}


def load_expected(release, manifest_sha256, configuration, configuration_sha256):
    # Host-only imports. The guest program never imports repository modules or
    # reads the host bundle; only validated public fingerprints are transmitted.
    from tools import guest_artifacts as artifacts
    identity_value = artifacts.template_identity(release, manifest_sha256,
                                                  configuration, configuration_sha256)
    manifest = artifacts.fingerprint(Path(release) / 'manifest.json', artifacts.MANIFEST_LIMIT)
    require(manifest['sha256'] == manifest_sha256)
    expected = {**identity_value['artifacts'], 'configuration': identity_value['configuration'],
                'manifest': manifest}
    validate_expected(expected)
    return identity_value, expected


def command(expected):
    validate_expected(expected)
    with Path(__file__).open('rb') as source_file:
        source = source_file.read(PROGRAM_LIMIT + 1)
    require(len(source) <= PROGRAM_LIMIT)
    # -I ignores guest PYTHON* overrides and user site modules. Assertions are
    # not used anywhere in this observer; failure checks survive optimization.
    return ' '.join(shlex.quote(arg) for arg in
                    ('/usr/bin/python3', '-I', '-c', source.decode(), json.dumps(expected, sort_keys=True)))


def receipt(result, expected):
    require(result.exit_code == 0 and not result.stderr
            and len(result.stdout.encode()) <= REPORT_LIMIT)
    report = json.loads(result.stdout, object_pairs_hook=unique)
    require(type(report) is dict and report.keys() == {'schema_version', 'files', 'daemon'}
            and type(report['schema_version']) is int and report['schema_version'] == 1)
    validate_expected(report['files'])
    require(report['files'] == expected)
    daemon = report['daemon']
    require(type(daemon) is dict and daemon.keys() == {'pid', 'start_ticks', 'socket_inode'})
    require(all(type(value) is int and 0 < value < 2**64 for value in daemon.values())
            and daemon['pid'] <= 4_194_304)
    return report


if __name__ == '__main__':
    try:
        require(len(sys.argv) == 2 and len(sys.argv[1].encode()) <= REPORT_LIMIT)
        print(json.dumps(observe(json.loads(sys.argv[1], object_pairs_hook=unique)), sort_keys=True))
    except Exception:
        print('guest_identity_failed', file=sys.stderr)
        sys.exit(1)
