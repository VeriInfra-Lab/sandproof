#!/usr/bin/env python3
"""Host-only, one-job QEMU runner. No host mounts, SSH forwarding or guest KVM.

The administrative GitHub credential belongs to the caller, not this tool or VM.
Pass only a one-job encoded JIT configuration on stdin. Runtime files are private
to this installation; only files created by this invocation are removed.
"""
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import selectors
import signal
import stat
import subprocess
import sys
import time
import uuid

ROOT = Path('/var/lib/sandproof-ci')
ROLES = {'rust': (4096, 4, 32), 'sdk': (3072, 4, 32),
         'harbor': (2048, 2, 32), 'title': (768, 1, 32),
         'policy': (768, 1, 32), 'bootstrap': (4096, 4, 32)}
PROXY = 'http://10.0.2.100:3128'


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


def cloud_config(role, jit='', pins=None):
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
            file_entry('/root/ci-bootstrap.sh', script, '0700')]
        result['write_files'].append(file_entry('/opt/sandproof-ci/check-isolation.py',
            Path(__file__).with_name('check_ci_isolation.py').read_text(), '0555'))
        result['runcmd'] = [['bash', '/root/ci-bootstrap.sh']]
    else:
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
        result['runcmd'] = [['chown', '-R', 'runner:runner', '/opt/actions-runner'],
            ['systemctl', 'daemon-reload'], ['bash', '-c',
            'if runuser -u runner -- python3 /opt/sandproof-ci/check-isolation.py; then '
            'systemctl start --no-block sandproof-ci-job.service; else poweroff; fi']]
    return result


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


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


def wait_with_bounded_logs(process, directory, timeout, limit=8 * 1024 ** 2):
    """Drain both pipes, bounding log bytes without limiting writable disks."""
    started = time.monotonic()
    with selectors.DefaultSelector() as selector:
        with (directory / 'console.log').open('wb') as console, (directory / 'qemu.log').open('wb') as errors:
            counts = {console: 0, errors: 0}
            selector.register(process.stdout, selectors.EVENT_READ, console)
            selector.register(process.stderr, selectors.EVENT_READ, errors)
            while selector.get_map():
                if time.monotonic() - started > timeout:
                    raise subprocess.TimeoutExpired('owned CI VM', timeout)
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
            process.wait(timeout=max(1, timeout - (time.monotonic() - started)))


def valid_run_id(value):
    if not re.fullmatch(r'[0-9a-f]{32}', value):
        raise ValueError('invalid owned run identity')
    return value


def cleanup_work(directory):
    # The supervisor only calls this after systemd confirms the whole worker
    # cgroup is gone. Refuse unfamiliar entries/symlinks, never recursive rm.
    allowed = {'user-data', 'meta-data', 'seed.img', 'work.qcow2',
               'console.log', 'qemu.log', 'report.json', 'host.log'}
    if directory.is_symlink() or directory.stat().st_uid != 0:
        raise ValueError('unsafe owned run directory')
    for entry in directory.iterdir():
        if entry.name not in allowed or not stat.S_ISREG(entry.lstat().st_mode):
            raise ValueError('unexpected owned run entry')
    for name in ('user-data', 'meta-data', 'seed.img', 'work.qcow2'):
        path = directory / name
        if path.exists():
            path.unlink()


def run(role, jit, pins, timeout, run_id=None):
    if os.geteuid() != 0:
        raise ValueError('host provisioning must run as root; guest jobs never do')
    os.umask(0o077)
    account = pwd.getpwnam('sandproof-ci-vm')
    if ROOT.is_symlink() or ROOT.stat().st_uid != 0:
        raise ValueError('unsafe installation root')
    source = ROOT / 'images' / ('ubuntu-noble-source.qcow2' if role == 'bootstrap' else 'golden.qcow2')
    expected = pins['cloud_sha256'] if role == 'bootstrap' else pins['golden_sha256']
    if source.is_symlink() or digest(source) != expected:
        raise ValueError('base image identity mismatch')
    info = subprocess.run(['/usr/bin/qemu-img', 'info', '--output=json', str(source)],
                          check=True, capture_output=True, text=True)
    validate_disk_capacity(role, json.loads(info.stdout)['virtual-size'])
    launcher_sha256 = digest(Path(__file__))
    managed = run_id is not None
    run_id = valid_run_id(run_id or uuid.uuid4().hex)
    # At most one VM per fixed role; the five ordinary jobs can run in parallel.
    lock = (ROOT / ('role-' + role + '.lock')).open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
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
    started = time.time()
    try:
        config = cloud_config(role, jit, pins)
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
        wait_with_bounded_logs(process, directory, timeout)
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
        report = {'id': run_id, 'role': role, 'started': started, 'finished': time.time(),
                  'vm_exited': process is not None and process.poll() is not None,
                  'provisioning_completed': success, 'base_sha256': expected,
                  'launcher_sha256': launcher_sha256,
                  'memory_mib': ROLES[role][0], 'cpus': ROLES[role][1]}
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
    args = parser.parse_args()
    if not 60 <= args.timeout <= 7200:
        parser.error('timeout must be between 60 and 7200 seconds')
    if (int(args.jit_stdin) + int(args.jit_file is not None)) != int(args.role != 'bootstrap'):
        parser.error('job VMs require exactly one private JIT source')
    try:
        pins = json.loads(args.pins.read_text())
        jit = sys.stdin.read(131073).strip() if args.jit_stdin else ''
        if args.jit_file:
            details = args.jit_file.lstat()
            if not stat.S_ISREG(details.st_mode) or details.st_uid != 0 or details.st_mode & 0o077:
                raise ValueError('JIT file must be root-private')
            with args.jit_file.open() as stream:
                jit = stream.read(131073).strip()
        run(args.role, jit, pins, args.timeout, args.run_id)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print('CI VM provisioning failed: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
