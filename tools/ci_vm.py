#!/usr/bin/env python3
"""Host-only, one-job QEMU runner. No host mounts, SSH forwarding or guest KVM.

The administrative GitHub credential belongs to the caller, not this tool or VM.
Pass only a one-job encoded JIT configuration on stdin. Runtime files are private
to this installation; only files created by this invocation are removed.
"""
import argparse
import base64
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import selectors
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import time
import uuid

ROOT = Path('/var/lib/sandproof-ci')
ROLES = {'rust': (4096, 4, 32), 'sdk': (3072, 4, 32),
         'harbor': (2048, 2, 32), 'title': (768, 1, 32),
         'policy': (768, 1, 32), 'bootstrap': (4096, 4, 32),
         'preheat': (4096, 4, 32)}
PREPARATION_ROLES = {'bootstrap', 'preheat'}
MAX_ROLE_WORKERS = 2
PROXY = 'http://10.0.2.100:3128'
RUNNER_JOBS = {'rust': 'rust', 'sdk': 'sdk-contract', 'harbor': 'harbor-inputs',
               'title': 'pr-title', 'policy': 'pr-policy'}
RUNNER_WAIT_SECONDS = 300
RUNNER_JOB_SECONDS = dict.fromkeys(RUNNER_JOBS, 1800) | {'rust': 2100}
# Independent systemd ceiling includes both phases and bounded own cleanup.
# Rust's 30-minute Actions deadline has five minutes for runner reporting/exit.
RUNNER_UNIT_SECONDS = {role: RUNNER_WAIT_SECONDS + seconds + 100
                       for role, seconds in RUNNER_JOB_SECONDS.items()}


def valid_slot(role, slot):
    # Keep the original lock for slot one, including an already running worker
    # during an upgrade. Bootstrap remains a separate, single-instance operation.
    allowed = {role} | ({role + '-2'} if role in ROLES and role not in PREPARATION_ROLES else set())
    if role not in ROLES or slot not in allowed:
        raise ValueError('invalid role slot')
    return slot


def lock_slot(role, slot):
    lock = (ROOT / ('role-' + valid_slot(role, slot) + '.lock')).open('a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BaseException:
        lock.close()
        raise
    return lock


def qemu_command(directory, role):
    memory, cpus, _ = ROLES[role]
    # restrict disables ordinary guest-to-host/external connectivity. Only this
    # fixed guestfwd can reach the host's domain/port restricted CI proxy.
    network = ('user,id=ci,restrict=on,guestfwd=tcp:10.0.2.100:3128-'
               'cmd:/usr/bin/socat - TCP:127.0.0.1:3129')
    return ['/usr/bin/qemu-system-x86_64', '-enable-kvm', '-cpu', 'host,-vmx,-svm',
            '-machine', 'q35', '-m', str(memory), '-mem-prealloc', '-smp', str(cpus),
            '-nodefaults', '-no-reboot', '-display', 'none', '-monitor', 'none',
            '-serial', 'stdio',
            '-drive', 'file=' + str(directory / 'work.qcow2') + ',if=virtio,format=qcow2',
            '-drive', 'file=' + str(directory / 'seed.img') + ',if=virtio,format=raw,readonly=on',
            '-netdev', network, '-device', 'virtio-net-pci,netdev=ci',
            # guestfwd invokes a fixed socat helper. exec must remain allowed;
            # setpriv below supplies OS-enforced no-new-privileges instead.
            '-sandbox', 'on,obsolete=deny,elevateprivileges=allow,spawn=allow,resourcecontrol=deny']


def file_entry(path, content, permissions='0644', owner='root:root'):
    return {'path': path, 'content': content, 'permissions': permissions, 'owner': owner}


# This root recipe runs before the unprivileged listener, only in its disposable
# overlay. It never accepts a PR-supplied path, image, device or shell command.
COW_SCRATCH_ROOT = '/var/lib/sandproof-ci-cow'
COW_SCRATCH_SETUP = r"""import json, os, pwd, stat, subprocess, sys
from pathlib import Path

ROOT = Path('/var/lib/sandproof-ci-cow')
SIZE = 1024 ** 3
PROBE = r'''import fcntl, json, os, stat, subprocess, sys, tempfile
from pathlib import Path
if sys.flags.optimize != 0:
    raise ValueError('scratch probe requires enabled assertions')
root = Path('/var/lib/sandproof-ci-cow/mount/tmp')
assert os.geteuid() != 0
info = root.stat()
assert info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == 0o700
with tempfile.TemporaryDirectory(dir=root) as directory:
    directory = Path(directory)
    source, target = directory / 'source', directory / 'target'
    flags = os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    with os.fdopen(os.open(source, flags, 0o600), 'wb+') as src:
        src.write(b'original' * 512)
        src.flush()
        os.fsync(src.fileno())
        with os.fdopen(os.open(target, flags, 0o600), 'wb+') as dst:
            fcntl.ioctl(dst.fileno(), 0x40049409, src.fileno())
            os.fsync(dst.fileno())
            a, b = os.fstat(src.fileno()), os.fstat(dst.fileno())
            assert a.st_dev == b.st_dev and a.st_ino != b.st_ino
            assert a.st_nlink == b.st_nlink == 1
            assert dst.read() == b'original' * 512
            dst.seek(0)
            dst.write(b'T')
            dst.flush()
            src.seek(0)
            assert src.read(1) == b'o'
            src.seek(1)
            src.write(b'S')
            src.flush()
            dst.seek(0)
            assert dst.read(2) == b'Tr'
    executable = directory / 'executable'
    executable.write_text('#!/bin/sh\nexit 0\n')
    executable.chmod(0o700)
    subprocess.run([str(executable)], check=True, timeout=10)
print(json.dumps({'runner_uid': os.geteuid(), 'python_optimize': sys.flags.optimize,
                  'clone_and_write_isolation': True,
                  'executable_tmpdir': True}))
'''


def run():
    if sys.flags.optimize != 0:
        raise ValueError('scratch preparation requires enabled assertions')
    if os.geteuid() != 0:
        raise ValueError('scratch preparation requires the trusted guest root recipe')
    for parent in [ROOT.parent, *ROOT.parent.parents]:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
            raise ValueError('scratch ancestor is not a protected root directory')
    os.umask(0o077)
    ROOT.mkdir(mode=0o711)  # Existing entries are never adopted or reformatted.
    ROOT.chmod(0o711)
    image, mount = ROOT / 'scratch.img', ROOT / 'mount'
    mount.mkdir(mode=0o700)
    fd = os.open(image, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.posix_fallocate(fd, 0, SIZE)
        os.fsync(fd)
        def identity():
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != 0
                    or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1
                    or info.st_size != SIZE):
                raise ValueError('scratch image identity changed')
            current = image.lstat()
            if (current.st_dev, current.st_ino) != (info.st_dev, info.st_ino):
                raise ValueError('scratch image name changed')
            return (info.st_dev, info.st_ino)
        original = identity()
        subprocess.run(['/usr/sbin/mkfs.xfs', '-f', '-K', '-m', 'reflink=1',
                        '/proc/self/fd/' + str(fd)], pass_fds=(fd,), check=True, timeout=30)
        if identity() != original:
            raise ValueError('scratch format changed the image identity')
        os.fsync(fd)
        subprocess.run(['/usr/bin/mount', '-o', 'loop,prjquota,nodev,nosuid',
                        str(image), str(mount)], check=True, timeout=30)
        if identity() != original:
            raise ValueError('scratch mount changed the image identity')
        filesystem = json.loads(subprocess.run(['/usr/bin/findmnt', '--json',
            '--target', str(mount), '--output', 'TARGET,FSTYPE,OPTIONS'],
            check=True, capture_output=True, text=True, timeout=10).stdout)['filesystems']
        if len(filesystem) != 1 or filesystem[0]['target'] != str(mount):
            raise ValueError('scratch mount observation is ambiguous')
        entry = filesystem[0]
        options = set(entry['options'].split(','))
        if (entry['fstype'] != 'xfs' or not {'rw', 'nodev', 'nosuid'} <= options
                or not options & {'prjquota', 'pquota'} or 'noexec' in options):
            raise ValueError('scratch filesystem or mount options do not meet the job contract')
        mount.chmod(0o711)
        tmp = mount / 'tmp'
        tmp.mkdir(mode=0o700)
        runner = pwd.getpwnam('runner')
        os.chown(tmp, runner.pw_uid, runner.pw_gid)
        probe = subprocess.run(['/usr/sbin/runuser', '-u', 'runner', '--',
            '/usr/bin/python3', '-I', '-c', PROBE], check=True, capture_output=True,
            text=True, timeout=30)
        witness = json.loads(probe.stdout)
        if witness != {'runner_uid': runner.pw_uid, 'python_optimize': 0,
                       'clone_and_write_isolation': True,
                       'executable_tmpdir': True}:
            raise ValueError('scratch runner capability probe did not complete')
        info = os.fstat(fd)
        print('SANDPROOF_CI_COW_READY ' + json.dumps(dict(witness,
            preparation_python_optimize=sys.flags.optimize,
            logical_bytes=SIZE, allocated_bytes=info.st_blocks * 512,
            filesystem=entry['fstype'], mount_options=sorted(options))), flush=True)
    finally:
        os.close(fd)


if __name__ == '__main__':
    run()
"""


def cloud_config(role, jit='', pins=None, dependency_bundle=None, completion_nonce=None):
    env = ('http_proxy=' + PROXY + '\nhttps_proxy=' + PROXY + '\n'
           'HTTP_PROXY=' + PROXY + '\nHTTPS_PROXY=' + PROXY + '\n'
           'no_proxy=localhost,127.0.0.1\nNO_PROXY=localhost,127.0.0.1\n'
           'CARGO_BUILD_JOBS=2\nRUST_TEST_THREADS=2\n'
           'PATH=/home/runner/.cargo/bin:/home/runner/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin\n')
    result = {'hostname': 'sandproof-ci-' + role, 'ssh_pwauth': False,
              'disable_root': True,
              'users': [{'name': 'runner', 'groups': 'users',
              'sudo': None, 'lock_passwd': True, 'shell': '/bin/bash'}],
              'write_files': [file_entry('/etc/sandproof-ci.env', env)],
              'runcmd': []}
    if role == 'bootstrap':
        if pins is None:
            raise ValueError('bootstrap requires verified source pins')
        script = Path(__file__).with_name('ci_vm_bootstrap.sh').read_text()
        result['write_files'] += [
            file_entry('/etc/apt/apt.conf.d/90-sandproof-ci',
                       'Acquire::http::Proxy "' + PROXY + '";\nAcquire::https::Proxy "' + PROXY + '";\n'),
            file_entry('/root/ci-pins.json', json.dumps(pins), '0600'),
            file_entry('/root/ci-bootstrap.sh', script, '0700'),
            file_entry('/opt/sandproof-ci/install-uv.sh',
                       Path(__file__).with_name('ci_install_uv.sh').read_text(), '0555')]
        result['write_files'].append(file_entry('/opt/sandproof-ci/check-isolation.py',
            Path(__file__).with_name('check_ci_isolation.py').read_text(), '0555'))
        result['runcmd'] = [['bash', '/root/ci-bootstrap.sh']]
    elif role == 'preheat':
        if (pins is None or dependency_bundle is None or jit
                or not isinstance(completion_nonce, str)
                or not re.fullmatch(r'[0-9a-f]{64}', completion_nonce)):
            raise ValueError('preheat requires approved inputs and private completion identity, never JIT')
        input_sha = dependency_bundle['input_sha256']
        if not isinstance(input_sha, str) or not re.fullmatch(r'[0-9a-f]{64}', input_sha):
            raise ValueError('invalid dependency receipt identity')
        script = ('#!/bin/bash\nset -euo pipefail\ntrap "/usr/sbin/poweroff" EXIT\n'
                  'set -a\nsource /etc/sandproof-ci.env\nset +a\n'
                  '/usr/sbin/runuser -u runner -- python3 /opt/sandproof-ci/check-isolation.py\n'
                  'bash /opt/sandproof-ci/install-uv.sh /root/ci-pins.json\n'
                  'if test -e /usr/local/lib/sandproof-ci/redis-7.2.16; then\n'
                  '  python3 /opt/sandproof-ci/ci_redis.py check >/dev/null\n'
                  'else\n'
                  '  redis_stage="$(mktemp -d /var/tmp/sandproof-ci-redis.XXXXXXXX)"\n'
                  '  chown runner:runner "$redis_stage"\n'
                  '  /usr/sbin/runuser -u runner -- python3 /opt/sandproof-ci/ci_redis.py '
                  'build --archive /opt/sandproof-ci/redis-7.2.16.tar.gz --output "$redis_stage"\n'
                  '  python3 /root/stop-preparation-processes.py\n'
                  '  python3 /opt/sandproof-ci/ci_redis.py seal --stage "$redis_stage"\n'
                  'fi\n'
                  'python3 /root/tool-inventory.py > /root/tools-before.json\n'
                  '/usr/sbin/runuser -u runner -- python3 /opt/sandproof-ci/ci_dependencies.py '
                  'warmup --input /opt/sandproof-ci/dependency-input.json\n'
                  'python3 /root/stop-preparation-processes.py\n'
                  'python3 /root/tool-inventory.py > /root/tools-after.json\n'
                  'cmp /root/tools-before.json /root/tools-after.json\n'
                  '/usr/sbin/runuser -u runner -- python3 /opt/sandproof-ci/check-isolation.py\n'
                  'rm -- /opt/sandproof-ci/dependency-input.json /root/ci-pins.json '
                  '/root/tools-before.json /root/tools-after.json /root/tool-inventory.py '
                  '/root/stop-preparation-processes.py /root/ci-preheat.sh\n'
                  'rm -- /opt/sandproof-ci/redis-7.2.16.tar.gz\n'
                  'printf "%s\\n" "SANDPROOF_CI_DEPENDENCIES_SEALED ' + input_sha + ' '
                  + completion_nonce + '"\n')
        # Root-only receipt: dependency build backends run as runner and cannot
        # read this nonce or fabricate the root recipe's exact completion line.
        inventory = '''import hashlib, json, os, pwd, stat
from pathlib import Path
rows = []
cache_dirs = {'/home/runner/.cache/uv', '/home/runner/.cargo/registry', '/home/runner/.cargo/git'}
cache_files = {'/home/runner/.cargo/.package-cache', '/home/runner/.cargo/.package-cache-mutate',
               '/home/runner/.cargo/.global-cache', '/home/runner/.cargo/.global-cache-wal',
               '/home/runner/.cargo/.global-cache-shm', '/home/runner/.cache'}
uid = pwd.getpwnam('runner').pw_uid
for value in sorted(cache_dirs | cache_files):
    path = Path(value)
    if path.exists() or path.is_symlink():
        info = path.lstat()
        wanted_directory = value in cache_dirs or value == '/home/runner/.cache'
        if (info.st_uid != uid or stat.S_ISLNK(info.st_mode)
                or (wanted_directory and not stat.S_ISDIR(info.st_mode))
                or (not wanted_directory and not stat.S_ISREG(info.st_mode))):
            raise ValueError('cache entry cannot redirect preparation outside its owned path')
for base in ('/opt/actions-runner', '/home/runner', '/usr/local/bin/uv'):
    root = Path(base)
    paths = [root]
    if root.is_dir():
        for directory, dirs, files in os.walk(root, followlinks=False):
            dirs[:] = sorted(name for name in dirs if str(Path(directory) / name) not in cache_dirs)
            paths.extend(Path(directory) / name for name in sorted(dirs + files))
    for path in paths:
        if str(path) in cache_files:
            continue
        info = path.lstat()
        if stat.S_ISLNK(info.st_mode):
            value = ['link', str(path.readlink())]
        elif stat.S_ISREG(info.st_mode):
            with path.open('rb') as stream:
                value = ['file', hashlib.file_digest(stream, 'sha256').hexdigest()]
        elif stat.S_ISDIR(info.st_mode):
            value = ['directory']
        else:
            raise ValueError('unexpected executable-tree entry')
        rows.append([str(path), stat.S_IMODE(info.st_mode), info.st_uid, info.st_gid, value])
print(json.dumps(rows, separators=(',', ':')))
'''
        quiesce = '''import os, pwd, signal, subprocess, time
uid = pwd.getpwnam('runner').pw_uid
def processes():
    result = subprocess.run(['pgrep', '-u', str(uid)], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise ValueError('preparation process inventory unknown')
    return [int(value) for value in result.stdout.split()]
for action in (signal.SIGTERM, signal.SIGKILL):
    for pid in processes():
        try:
            os.kill(pid, action)
        except ProcessLookupError:
            pass
    for _ in range(25):
        if not processes():
            break
        time.sleep(0.2)
if processes():
    raise ValueError('preparation descendants still active; refusing image seal')
'''
        result['write_files'] += [
            file_entry('/root/ci-pins.json', json.dumps(pins), '0600'),
            file_entry('/root/ci-preheat.sh', script, '0700'),
            file_entry('/root/tool-inventory.py', inventory, '0500'),
            file_entry('/root/stop-preparation-processes.py', quiesce, '0500'),
            file_entry('/opt/sandproof-ci/install-uv.sh',
                       Path(__file__).with_name('ci_install_uv.sh').read_text(), '0555'),
            file_entry('/opt/sandproof-ci/ci_dependencies.py',
                       Path(__file__).with_name('ci_dependencies.py').read_text(), '0555'),
            file_entry('/opt/sandproof-ci/ci_git.py',
                       Path(__file__).with_name('ci_git.py').read_text(), '0555'),
            file_entry('/opt/sandproof-ci/ci_redis.py',
                       Path(__file__).with_name('ci_redis.py').read_text(), '0555'),
            dict(file_entry('/opt/sandproof-ci/redis-7.2.16.tar.gz',
                            dependency_bundle['redis_archive'], '0444'), encoding='b64'),
            file_entry('/opt/sandproof-ci/dependency-input.json',
                       json.dumps(dependency_bundle), '0444'),
            file_entry('/opt/sandproof-ci/dependency-manifest.json',
                       json.dumps(dependency_bundle['manifest']), '0444'),
            file_entry('/opt/sandproof-ci/check-isolation.py',
                       Path(__file__).with_name('check_ci_isolation.py').read_text(), '0555')]
        result['runcmd'] = [['bash', '/root/ci-preheat.sh']]
    else:
        if dependency_bundle is not None or completion_nonce is not None:
            raise ValueError('ordinary jobs cannot prepare dependency images')
        if not re.fullmatch(r'[A-Za-z0-9+/=]{16,131072}', jit):
            raise ValueError('invalid opaque JIT configuration')
        # Never put the JIT configuration into host argv, a report, or stdout.
        base64.b64decode(jit, validate=True)
        start = ('#!/bin/bash\nset -euo pipefail\ncd /opt/actions-runner\n'
                 'exec ./run.sh --jitconfig "$(< /home/runner/.jitconfig)"\n')
        unit = ('[Unit]\nAfter=network-online.target cloud-final.service\n'
                'Wants=network-online.target\n[Service]\nUser=runner\nGroup=runner\n'
                'EnvironmentFile=/etc/sandproof-ci.env\nWorkingDirectory=/opt/actions-runner\n'
                'ExecStart=/opt/actions-runner/start-jit.sh\n'
                'ExecStopPost=+/usr/sbin/poweroff\nNoNewPrivileges=true\n'
                'ProtectSystem=full\nProtectHome=false\nPrivateTmp=true\n'
                'RestrictSUIDSGID=true\nCapabilityBoundingSet=\n'
                'StandardOutput=journal+console\nStandardError=journal+console\n'
                '[Install]\nWantedBy=multi-user.target\n')
        result['write_files'] += [
            file_entry('/opt/sandproof-ci/check-isolation.py',
                       Path(__file__).with_name('check_ci_isolation.py').read_text(), '0555'),
            file_entry('/home/runner/.jitconfig', jit, '0600', 'runner:runner'),
            file_entry('/opt/actions-runner/start-jit.sh', start, '0755'),
            file_entry('/etc/systemd/system/sandproof-ci-job.service', unit)]
        preparation = ''
        if role in {'rust', 'sdk'}:
            result['write_files'][0]['content'] += 'TMPDIR=' + COW_SCRATCH_ROOT + '/mount/tmp\n'
            result['write_files'].append(file_entry('/root/ci-cow-scratch.py',
                                                   COW_SCRATCH_SETUP, '0500'))
            preparation = '/usr/bin/python3 -I /root/ci-cow-scratch.py && '
        result['runcmd'] = [['chown', '-R', 'runner:runner', '/opt/actions-runner'],
            ['systemctl', 'daemon-reload'], ['bash', '-c',
            'if ' + preparation + 'runuser -u runner -- python3 /opt/sandproof-ci/check-isolation.py; then '
            'systemctl start --no-block sandproof-ci-job.service; else poweroff; fi']]
    return result


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_image(role, pins):
    if role == 'bootstrap':
        return ROOT / 'images' / 'ubuntu-noble-source.qcow2', pins['cloud_sha256']
    expected = pins['golden_sha256']
    name = pins.get('golden_image', 'golden.qcow2')
    if (not isinstance(expected, str) or not re.fullmatch(r'[0-9a-f]{64}', expected)
            or name not in ('golden.qcow2', 'golden-' + expected + '.qcow2')):
        raise ValueError('invalid immutable golden image identity')
    return ROOT / 'images' / name, expected


@contextmanager
def preparation_guard(identity):
    """Claim only our maintenance markers; never terminate in-flight jobs."""
    state_root = ROOT / 'supervisor'
    if (os.geteuid() != 0 or state_root.is_symlink() or state_root.stat().st_uid != 0
            or state_root.stat().st_mode & 0o077):
        raise ValueError('preparation admission must be root controlled')
    marker = json.dumps({'owner': 'ci-dependencies-' + valid_run_id(identity)}) + '\n'
    paths = [state_root / 'paused', state_root / 'exclusive-host']
    def locked():
        return open(state_root / 'admission.lock', 'a',
                    opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600))
    created = []
    with locked() as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if any(path.exists() or path.is_symlink() for path in paths):
            raise ValueError('another maintenance or E2E owner holds admission')
        state = state_root / 'state.json'
        if state.is_symlink():
            raise ValueError('ordinary CI state identity unknown')
        if state.exists():
            info = state.stat()
            if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or json.loads(state.read_text()) != {}:
                raise ValueError('ordinary CI leases must finish before preparation')
        units = subprocess.run(['systemctl', 'list-units', '--plain', '--no-legend',
            '--state=active,activating,deactivating', 'sandproof-ci-worker-*.service'],
            check=True, capture_output=True, text=True, timeout=15)
        if units.stdout.strip():
            raise ValueError('ordinary CI workers must finish before preparation')
        fields = dict(line.split(':', 1) for line in Path('/proc/meminfo').read_text().splitlines())
        if int(fields['MemAvailable'].split()[0]) < (4096 + 2048 + 512) * 1024:
            raise ValueError('dependency preparation memory reserve unavailable')
        if shutil.disk_usage(ROOT).free < 72 * 1024 ** 3:
            raise ValueError('dependency preparation disk reserve unavailable')
        try:
            for path in paths:
                with open(path, 'x', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
                    stream.write(marker)
                    stream.flush()
                    os.fsync(stream.fileno())
                created.append(path)
        except BaseException:
            for path in created:
                path.unlink()
            raise
    try:
        yield
    finally:
        # run() deletes this disk only after its VM process is confirmed gone.
        if (ROOT / 'runs' / identity / 'work.qcow2').exists():
            raise ValueError('preparation VM cleanup unknown; maintenance retained')
        with locked() as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            for path in created:
                info = path.lstat()
                if (not stat.S_ISREG(info.st_mode) or info.st_uid != 0
                        or path.read_text() != marker):
                    raise ValueError('maintenance owner changed; refusing to remove marker')
            for path in created:
                path.unlink()


def publish_preheated(directory, pins, input_sha):
    """Publish immutable standalone backing plus candidate pins, not live pins."""
    temporary = directory / 'prepared.qcow2'
    created = False
    try:
        with temporary.open('xb'):
            created = True
        subprocess.run(['/usr/bin/qemu-img', 'convert', '-O', 'qcow2',
                        str(directory / 'work.qcow2'), str(temporary)], check=True)
        result = subprocess.run(['/usr/bin/qemu-img', 'info', '--output=json', str(temporary)],
                                check=True, capture_output=True, text=True)
        info = json.loads(result.stdout)
        if info.get('backing-filename') or info.get('format') != 'qcow2':
            raise ValueError('prepared golden must be standalone qcow2')
        with temporary.open('rb') as stream:
            os.fsync(stream.fileno())
        temporary.chmod(0o444)
        image_sha = digest(temporary)
        target = ROOT / 'images' / ('golden-' + image_sha + '.qcow2')
        os.link(temporary, target)  # atomic no-replace; never replace old backing
        candidate = dict(pins, golden_sha256=image_sha, golden_image=target.name,
                         dependency_input_sha256=input_sha)
        candidate_path = directory / 'prepared-pins.json'
        with open(candidate_path, 'x', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
            json.dump(candidate, stream)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        for parent in (target.parent, directory):
            fd = os.open(parent, os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        return candidate_path
    finally:
        if created and temporary.exists():
            temporary.unlink()


def validate_disk_capacity(role, virtual_size):
    # A sparse overlay must not truncate the trusted base's partition table.
    if virtual_size <= 0 or virtual_size > ROLES[role][2] * 1024 ** 3:
        raise ValueError('role disk budget is smaller than its base image')


def stop_owned(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=15)


class RunnerDeadline:
    """One waiting phase and at most one execution budget; never a success receipt.

    The fixed runner emits its start line before executing repository steps.
    Later task-controlled output cannot restart this budget or exceed the hard
    ceiling. Wrong-role, partial or oversized lines never grant an extension.
    Preparation has no job marker and retains its original absolute timeout.
    """
    def __init__(self, timeout, job_name=None, clock=None):
        if job_name is not None and job_name not in RUNNER_JOBS.values():
            raise ValueError('unknown runner job')
        self.clock = clock or time.monotonic
        self.timeout, self.job_name = timeout, job_name
        self.started = self.clock()
        self.hard_deadline = self.started + timeout + (RUNNER_WAIT_SECONDS if job_name else 0)
        self.deadline = self.started + (RUNNER_WAIT_SECONDS if job_name else timeout)
        self.job_started = False
        self.pending = b''
        self.oversized_line = False
        self.pattern = re.compile(
            rb'\[\s*[0-9]+(?:\.[0-9]+)?\]\s+start-jit\.sh\[[0-9]+\]: '
            rb'[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}Z: Running job: '
            + re.escape((job_name or '').encode()))

    def remaining(self):
        remaining = min(self.deadline, self.hard_deadline) - self.clock()
        if remaining <= 0:
            phase = 'waiting for runner job' if self.job_name and not self.job_started else 'execution'
            raise subprocess.TimeoutExpired('owned CI VM ' + phase,
                RUNNER_WAIT_SECONDS if self.job_name and not self.job_started else self.timeout)
        return remaining

    def feed(self, data):
        if self.job_name is None or self.job_started:
            return
        self.remaining()  # A late marker cannot resurrect an expired wait.
        lines = (self.pending + data).replace(b'\r', b'\n').split(b'\n')
        if self.oversized_line:
            if len(lines) == 1:
                return  # Ignore the entire long line, not its marker-like tail.
            lines = lines[1:]
        self.oversized_line = len(lines[-1]) > 512
        self.pending = b'' if self.oversized_line else lines[-1]
        for line in lines[:-1]:
            if len(line) <= 512 and self.pattern.fullmatch(line.strip()):
                self.job_started = True
                self.pending = b''
                self.deadline = min(self.hard_deadline, self.clock() + self.timeout)
                return


def wait_with_bounded_logs(process, directory, timeout, limit=8 * 1024 ** 2, *, job_name=None):
    """Drain both pipes, bounding log bytes without limiting writable disks."""
    budget = RunnerDeadline(timeout, job_name)
    with selectors.DefaultSelector() as selector:
        with (directory / 'console.log').open('wb') as console, (directory / 'qemu.log').open('wb') as errors:
            counts = {console: 0, errors: 0}
            selector.register(process.stdout, selectors.EVENT_READ, console)
            selector.register(process.stderr, selectors.EVENT_READ, errors)
            while selector.get_map():
                budget.remaining()
                for key, _events in selector.select(timeout=1):
                    data = os.read(key.fd, 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                        continue
                    remaining = limit - counts[key.data]
                    key.data.write(data[:remaining])
                    key.data.flush()
                    counts[key.data] += min(remaining, len(data))
                    if len(data) > remaining:
                        raise ValueError('owned VM diagnostic limit exceeded')
                    if key.data is console:
                        budget.feed(data)
            process.wait(timeout=budget.remaining())


def valid_run_id(value):
    if not re.fullmatch(r'[0-9a-f]{32}', value):
        raise ValueError('invalid owned run identity')
    return value


def cleanup_work(directory):
    # The supervisor only calls this after systemd confirms the whole worker
    # cgroup is gone. Refuse unfamiliar entries/symlinks, never recursive rm.
    allowed = {'user-data', 'meta-data', 'seed.img', 'work.qcow2',
               'console.log', 'qemu.log', 'report.json', 'host.log', 'prepared-pins.json'}
    if directory.is_symlink() or directory.stat().st_uid != 0:
        raise ValueError('unsafe owned run directory')
    for entry in directory.iterdir():
        if entry.name not in allowed or not stat.S_ISREG(entry.lstat().st_mode):
            raise ValueError('unexpected owned run entry')
    for name in ('user-data', 'meta-data', 'seed.img', 'work.qcow2'):
        path = directory / name
        if path.exists():
            path.unlink()


def run(role, jit, pins, timeout, run_id=None, slot=None, dependency_bundle=None):
    slot = valid_slot(role, slot or role)
    if os.geteuid() != 0:
        raise ValueError('host provisioning must run as root; guest jobs never do')
    os.umask(0o077)
    account = pwd.getpwnam('sandproof-ci-vm')
    if ROOT.is_symlink() or ROOT.stat().st_uid != 0:
        raise ValueError('unsafe installation root')
    if (role == 'preheat') != (dependency_bundle is not None) or (role in PREPARATION_ROLES and jit):
        raise ValueError('preheat inputs and preparation role must match; no administrative JIT')
    source, expected = source_image(role, pins)
    if source.is_symlink() or digest(source) != expected:
        raise ValueError('base image identity mismatch')
    info = subprocess.run(['/usr/bin/qemu-img', 'info', '--output=json', str(source)],
                          check=True, capture_output=True, text=True)
    validate_disk_capacity(role, json.loads(info.stdout)['virtual-size'])
    launcher_sha256 = digest(Path(__file__))
    managed = run_id is not None
    run_id = valid_run_id(run_id or uuid.uuid4().hex)
    # The trusted supervisor budgets independent slots, not a global role lock.
    # Duplicate launches of the same slot still fail without touching its VM.
    lock = lock_slot(role, slot)
    parent = ROOT / 'runs'
    parent.mkdir(mode=0o711, exist_ok=True)
    directory = parent / run_id
    directory.mkdir(mode=0o711, exist_ok=managed)
    directory.chmod(0o711)
    if directory.is_symlink() or directory.stat().st_uid != 0 or set(p.name for p in directory.iterdir()) - {'host.log'}:
        raise ValueError('owned run directory must be new and root controlled')
    # QEMU can access its disks, but cannot replace directory entries. The
    # supervisor owns all metadata/logs and bounds pipe output before writing.
    process = None
    success = False
    candidate_path = None
    nonce = secrets.token_hex(32) if role == 'preheat' else None
    started = time.time()
    try:
        config = cloud_config(role, jit, pins, dependency_bundle, nonce)
        (directory / 'user-data').write_text('#cloud-config\n' + json.dumps(config))
        (directory / 'user-data').chmod(0o600)
        (directory / 'meta-data').write_text('instance-id: ci-' + run_id + '\n')
        subprocess.run(['/usr/bin/cloud-localds', str(directory / 'seed.img'),
                        str(directory / 'user-data'), str(directory / 'meta-data')], check=True,
                       stdout=subprocess.DEVNULL)
        subprocess.run(['/usr/bin/qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2',
                        '-b', str(source), str(directory / 'work.qcow2'),
                        str(ROLES[role][2]) + 'G'], check=True, stdout=subprocess.DEVNULL)
        for name in ('seed.img', 'work.qcow2'):
            os.chown(directory / name, account.pw_uid, account.pw_gid)
            (directory / name).chmod(0o600)
        command = ['/usr/sbin/runuser', '-u', 'sandproof-ci-vm', '--',
                   '/usr/bin/setpriv', '--no-new-privs'] + qemu_command(directory, role)
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        wait_with_bounded_logs(process, directory, timeout, job_name=RUNNER_JOBS.get(role))
        if process.returncode != 0:
            raise ValueError('QEMU failed; inspect private run ' + run_id)
        console = (directory / 'console.log').read_text(errors='replace')
        if role == 'bootstrap':
            if 'SANDPROOF_CI_IMAGE_READY' not in console:
                raise ValueError('bootstrap did not publish readiness; inspect private run ' + run_id)
            target = ROOT / 'images' / 'golden.qcow2'
            if target.exists():
                raise ValueError('refusing to overwrite existing golden image')
            # Convert to a standalone immutable base; never back future jobs by
            # a previous job's writable overlay.
            subprocess.run(['/usr/bin/qemu-img', 'convert', '-O', 'qcow2',
                            str(directory / 'work.qcow2'), str(target)], check=True)
            target.chmod(0o444)
            pins['golden_sha256'] = digest(target)
            (ROOT / 'pins.json').write_text(json.dumps(pins, indent=2) + '\n')
            (ROOT / 'pins.json').chmod(0o600)
        elif role == 'preheat':
            receipt = 'SANDPROOF_CI_DEPENDENCIES_SEALED ' + dependency_bundle['input_sha256'] + ' ' + nonce
            if not any(line.split(']: ', 1)[-1].strip() == receipt for line in console.splitlines()):
                raise ValueError('dependency recipe did not seal verified caches; inspect private run ' + run_id)
            candidate_path = publish_preheated(directory, pins, dependency_bundle['input_sha256'])
        elif 'SANDPROOF_CI_ISOLATION_OK' not in console:
            raise ValueError('pre-job isolation probe failed; inspect private run ' + run_id)
        elif 'Listening for Jobs' not in console:
            raise ValueError('runner never connected; inspect private run ' + run_id)
        success = True
    finally:
        if process is not None:
            stop_owned(process)
            process.stdout.close()
            process.stderr.close()
        # Preserve bounded diagnostics separately from executable work disks.
        report = {'id': run_id, 'role': role, 'slot': slot, 'started': started, 'finished': time.time(),
                  'vm_exited': process is not None and process.poll() is not None,
                  'provisioning_completed': success, 'base_sha256': expected,
                  'launcher_sha256': launcher_sha256,
                  'memory_mib': ROLES[role][0], 'cpus': ROLES[role][1]}
        if role == 'preheat':
            report['dependency_input_sha256'] = dependency_bundle['input_sha256']
            report['candidate_pins'] = str(candidate_path) if candidate_path is not None else None
        (directory / 'report.json').write_text(json.dumps(report) + '\n')
        cleanup_work(directory)
        print(json.dumps(report), flush=True)
        lock.close()


def main():
    def interrupted(_signum, _frame):
        raise InterruptedError('provisioning interrupted; own VM will be stopped')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGHUP, interrupted)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--role', choices=ROLES, required=True)
    parser.add_argument('--pins', type=Path, default=ROOT / 'pins.json')
    parser.add_argument('--timeout', type=int, default=3600)
    parser.add_argument('--jit-stdin', action='store_true')
    parser.add_argument('--jit-file', type=Path)
    parser.add_argument('--run-id', type=valid_run_id)
    parser.add_argument('--slot')
    parser.add_argument('--dependency-input', type=Path)
    args = parser.parse_args()
    if not 60 <= args.timeout <= 7200:
        parser.error('timeout must be between 60 and 7200 seconds')
    if (int(args.jit_stdin) + int(args.jit_file is not None)) != int(args.role not in PREPARATION_ROLES):
        parser.error('job VMs require exactly one private JIT source')
    if (args.role == 'preheat') != (args.dependency_input is not None):
        parser.error('only preheat requires --dependency-input')
    try:
        pins = json.loads(args.pins.read_text())
        jit = sys.stdin.read(131073).strip() if args.jit_stdin else ''
        if args.jit_file:
            details = args.jit_file.lstat()
            if not stat.S_ISREG(details.st_mode) or details.st_uid != 0 or details.st_mode & 0o077:
                raise ValueError('JIT file must be root-private')
            with args.jit_file.open() as stream:
                jit = stream.read(131073).strip()
        if args.role == 'preheat':
            if __package__:
                from .ci_dependencies import load_input
            else:
                from ci_dependencies import load_input
            identity = args.run_id or uuid.uuid4().hex
            bundle = load_input(args.dependency_input)
            with preparation_guard(identity):
                run(args.role, jit, pins, args.timeout, identity, args.slot, bundle)
        else:
            run(args.role, jit, pins, args.timeout, args.run_id, args.slot)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print('CI VM provisioning failed: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
