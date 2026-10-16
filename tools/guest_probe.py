"""Read-only pinned guest observation through the existing authenticated SDK.

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
import time

PLATFORM = '/opt/sandproof'
NAMES = {'service': 'sandproof-guest-service', 'worker': 'sandproof-guest-worker',
         'configuration': 'guest-config.json', 'manifest': 'guest-manifest.json'}
LIMITS = {'service': 256 * 1024**2, 'worker': 256 * 1024**2,
          'configuration': 1024**2, 'manifest': 16 * 1024}
PROGRAM_LIMIT = 24 * 1024
COMMAND_LIMIT = 32 * 1024
REPORT_LIMIT = 4096
PROCESS_LIMIT = 4096
FD_LIMIT = 4096
GO_PATH = '/usr/local/bin/envd'


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
        validate_file(value, LIMITS[role])


def validate_file(value, maximum):
    require(type(value) is dict and value.keys() == {'bytes', 'sha256'})
    require(type(value['bytes']) is int and 0 < value['bytes'] <= maximum)
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


def installed_file(path, expected, owner, mode):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = hashed(fd, expected, owner)
        require(identity(path.lstat()) == identity(info) and stat.S_IMODE(info.st_mode) == mode)
        return info
    finally:
        os.close(fd)


def installed(platform, expected, owner):
    return {role: installed_file(platform / name, expected[role], owner,
                0o500 if role in ('service', 'worker') else 0o400)
            for role, name in NAMES.items()}


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


def listener_inode(proc, port, *, go_wildcard=False):
    found = []
    tables = [('tcp', '00000000')]
    if go_wildcard:
        # Go's net.Listen("tcp", wildcard) may use a dual-stack socket even
        # when the configured address is 0.0.0.0. Rust keeps its IPv4 contract.
        tables.append(('tcp6', '0' * 32))
    for table, wildcard in tables:
        try:
            lines = small_file(proc / 'net' / table, 1024**2).splitlines()
        except FileNotFoundError:
            require(table == 'tcp6')  # IPv6 may be disabled in this kernel.
            continue
        for line in lines[1:]:
            fields = line.split()
            require(len(fields) >= 10)
            if fields[1] == f'{wildcard}:{port:04X}'.encode() and fields[3] == b'0A':
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
    argv = (str(platform / NAMES['service']) + '\0serve\0--config\0'
            + str(platform / NAMES['configuration']) + '\0').encode()
    return observe_process(proc, expected['service'], installed_service, owner, argv)


def observe_process(proc, expected, installed_service, owner, argv, *, go_wildcard=False):
    inode = listener_inode(proc, 49983, go_wildcard=go_wildcard)
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
                    actual = hashed(executable, expected, owner)
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
    require(len(matches) == 1
            and listener_inode(proc, 49983, go_wildcard=go_wildcard) == inode)
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
    return program_command(expected)


def program_command(expected):
    with Path(__file__).open('rb') as source_file:
        source = source_file.read(PROGRAM_LIMIT + 1)
    require(len(source) <= PROGRAM_LIMIT)
    # -I ignores guest PYTHON* overrides and user site modules. Assertions are
    # not used anywhere in this observer; failure checks survive optimization.
    result = ' '.join(shlex.quote(arg) for arg in
                     ('/usr/bin/python3', '-I', '-c', source.decode(), json.dumps(expected, sort_keys=True)))
    require(len(result.encode()) <= COMMAND_LIMIT)
    return result


def receipt(result, expected):
    require(result.exit_code == 0 and not result.stderr
            and len(result.stdout.encode()) <= REPORT_LIMIT)
    report = json.loads(result.stdout, object_pairs_hook=unique)
    require(type(report) is dict and report.keys() == {'schema_version', 'files', 'daemon'}
            and type(report['schema_version']) is int and report['schema_version'] == 1)
    validate_expected(report['files'])
    require(report['files'] == expected)
    validate_daemon(report['daemon'])
    return report


def validate_daemon(daemon):
    require(type(daemon) is dict and daemon.keys() == {'pid', 'start_ticks', 'socket_inode'})
    require(all(type(value) is int and 0 < value < 2**64 for value in daemon.values())
            and daemon['pid'] <= 4_194_304)


def validate_go(expected):
    # Reuse the same strict scalar bounds without allowing arbitrary guest
    # paths, command lines or ports in the observer's input.
    validate_file(expected, LIMITS['service'])


def load_go_expected():
    root = Path(__file__).resolve().parents[1]
    profile = json.loads((root / 'tests/system/base-inputs.json').read_text(), object_pairs_hook=unique)
    baseline = json.loads((root / 'references/v1-baseline.json').read_text(), object_pairs_hook=unique)
    envd = profile['envd']
    require(envd['source_commit'] == baseline['source_revisions']['e2b_runtime_behavior_reference'])
    expected = {key: envd[key] for key in ('bytes', 'sha256')}
    validate_go(expected)
    return {'source_commit': envd['source_commit'], 'version': envd['version'],
            'go_version': envd['go_version'], 'file': expected}


def go_command(expected):
    validate_go(expected)
    return program_command({'fixed_go': expected})


def observe_go(expected):
    validate_go(expected)
    require(sys.platform == 'linux' and os.geteuid() == 0)
    path = Path(GO_PATH)
    before = installed_file(path, expected, 0, 0o755)
    daemon = observe_process(Path('/proc'), expected, before, 0,
                             (GO_PATH + '\0-verbose\0').encode(), go_wildcard=True)
    require(identity(installed_file(path, expected, 0, 0o755)) == identity(before))
    return {'schema_version': 1, 'fixed_go': expected, 'daemon': daemon}


def go_receipt(result, expected):
    validate_go(expected)
    require(result.exit_code == 0 and not result.stderr
            and len(result.stdout.encode()) <= REPORT_LIMIT)
    report = json.loads(result.stdout, object_pairs_hook=unique)
    require(type(report) is dict and report.keys() == {'schema_version', 'fixed_go', 'daemon'}
            and type(report['schema_version']) is int and report['schema_version'] == 1
            and report['fixed_go'] == expected)
    validate_go(report['fixed_go'])
    validate_daemon(report['daemon'])
    return report


def validate_target(expected):
    if type(expected) is dict and expected.keys() == {'fixed_go'}:
        validate_go(expected['fixed_go'])
    else:
        validate_expected(expected)


def observe_target(expected):
    return (observe_go(expected['fixed_go']) if type(expected) is dict
            and expected.keys() == {'fixed_go'} else observe(expected))


def resource_command(expected, daemon):
    """Read-only sample of the exact previously observed Rust or Go daemon."""
    validate_target(expected)
    validate_daemon(daemon)
    return program_command({'resources': {'expected': expected, 'daemon': daemon}})


def proc_snapshot(name, directory):
    # A single bounded read avoids assembling smaps_rollup from multiple
    # changing snapshots. No fallback to inaccurate statm/VmRSS measurements.
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    try:
        raw = os.read(fd, 16 * 1024 + 1)
        require(len(raw) <= 16 * 1024 and not os.read(fd, 1))
        return raw
    finally:
        os.close(fd)


def proc_values(raw, fields):
    values = {}
    for line in raw.splitlines():
        key, separator, value = line.partition(b':')
        if separator and key in fields:
            require(key not in values)
            unit = fields[key]
            match = re.fullmatch(rb'([0-9]+)' + (rb'\s+kB' if unit == 1024 else b''), value.strip())
            require(match is not None)
            number = int(match[1]) * unit
            require(0 <= number < 2**64)
            values[key] = number
    require(values.keys() == fields.keys())
    return values


def validate_sample(sample):
    require(type(sample) is dict and sample.keys() == {
        'rss_bytes', 'reported_peak_rss_bytes', 'threads', 'open_fds', 'elapsed_ns'})
    require(all(type(value) is int and 0 <= value < 2**64 for value in sample.values()))
    require(0 < sample['threads'] <= PROCESS_LIMIT and 0 < sample['open_fds'] <= FD_LIMIT)


def read_resources(proc, daemon, *, go_wildcard=False):
    validate_daemon(daemon)
    pid, inode = daemon['pid'], daemon['socket_inode']
    directory = os.open(proc / str(pid), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        require(start_ticks(directory, pid) == daemon['start_ticks'] and owns_socket(directory, inode))
        executable = identity(os.stat('exe', dir_fd=directory))
        argv = small_file('cmdline', 4096, directory=directory)
        started = time.monotonic_ns()
        status = proc_values(proc_snapshot('status', directory),
                             {b'Pid': 1, b'Tgid': 1, b'Threads': 1, b'VmHWM': 1024})
        require(status[b'Pid'] == pid and status[b'Tgid'] == pid)
        memory = proc_values(proc_snapshot('smaps_rollup', directory), {b'Rss': 1024})
        fd = os.open('fd', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
        try:
            count = 0
            with os.scandir(fd) as entries:
                for entry in entries:
                    count += 1
                    require(count <= FD_LIMIT and entry.name.isascii() and entry.name.isdecimal())
        finally:
            os.close(fd)
        sample = dict(rss_bytes=memory[b'Rss'], reported_peak_rss_bytes=status[b'VmHWM'],
                      threads=status[b'Threads'], open_fds=count,
                      elapsed_ns=time.monotonic_ns() - started)
        validate_sample(sample)
        require(start_ticks(directory, pid) == daemon['start_ticks']
                and identity(os.stat('exe', dir_fd=directory)) == executable
                and small_file('cmdline', 4096, directory=directory) == argv
                and owns_socket(directory, inode)
                and listener_inode(proc, 49983, go_wildcard=go_wildcard) == inode)
        return sample
    finally:
        os.close(directory)


def observe_resources(request):
    require(type(request) is dict and request.keys() == {'expected', 'daemon'})
    expected, daemon = request['expected'], request['daemon']
    validate_target(expected)
    validate_daemon(daemon)
    before = observe_target(expected)
    require(before['daemon'] == daemon)
    sample = read_resources(Path('/proc'), daemon, go_wildcard=expected.keys() == {'fixed_go'})
    require(observe_target(expected) == before)
    return dict(schema_version=1, expected=expected, daemon=daemon, sample=sample)


def resource_receipt(result, expected, daemon):
    validate_target(expected)
    validate_daemon(daemon)
    require(result.exit_code == 0 and not result.stderr
            and len(result.stdout.encode()) <= REPORT_LIMIT)
    report = json.loads(result.stdout, object_pairs_hook=unique)
    require(type(report) is dict and report.keys() == {'schema_version', 'expected', 'daemon', 'sample'}
            and type(report['schema_version']) is int and report['schema_version'] == 1)
    validate_target(report['expected'])
    validate_daemon(report['daemon'])
    validate_sample(report['sample'])
    require(report['expected'] == expected and report['daemon'] == daemon)
    return report


if __name__ == '__main__':
    try:
        require(len(sys.argv) == 2 and len(sys.argv[1].encode()) <= REPORT_LIMIT)
        expected = json.loads(sys.argv[1], object_pairs_hook=unique)
        report = (observe_resources(expected['resources']) if type(expected) is dict
                  and expected.keys() == {'resources'} else observe_target(expected))
        print(json.dumps(report, sort_keys=True))
    except Exception:
        print('guest_identity_failed', file=sys.stderr)
        sys.exit(1)
