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
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path('/var/lib/sandproof-ci')
ROLES = {'rust': (4096, 4, 32), 'sdk': (3072, 4, 32),
         'harbor': (2048, 2, 24), 'title': (768, 1, 12),
         'policy': (768, 1, 12), 'bootstrap': (4096, 4, 32)}
PROXY = 'http://10.0.2.100:3128'


def qemu_command(directory, role):
    memory, cpus, _ = ROLES[role]
    # restrict disables ordinary guest-to-host/external connectivity. Only this
    # fixed guestfwd can reach the host's domain/port restricted CI proxy.
    network = ('user,id=ci,restrict=on,guestfwd=tcp:10.0.2.100:3128-'
               'cmd:/usr/bin/socat - TCP:127.0.0.1:3129')
    return ['/usr/bin/qemu-system-x86_64', '-enable-kvm', '-cpu', 'host',
            '-machine', 'q35', '-m', str(memory), '-smp', str(cpus),
            '-nodefaults', '-no-reboot', '-display', 'none', '-monitor', 'none',
            '-serial', 'file:' + str(directory / 'console.log'),
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
              'disable_root': True, 'groups': ['runner'],
              'users': [{'name': 'runner', 'groups': 'runner',
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
        result['runcmd'] = [['systemctl', 'daemon-reload'], ['bash', '-c',
            'if runuser -u runner -- python3 /opt/sandproof-ci/check-isolation.py; then '
            'systemctl start --no-block sandproof-ci-job.service; else poweroff; fi']]
    return result


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def stop_owned(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=15)


def run(role, jit, pins, timeout):
    if os.geteuid() != 0:
        raise ValueError('host provisioning must run as root; guest jobs never do')
    account = pwd.getpwnam('sandproof-ci-vm')
    if ROOT.is_symlink() or ROOT.stat().st_uid != 0:
        raise ValueError('unsafe installation root')
    source = ROOT / 'images' / ('ubuntu-noble-source.qcow2' if role == 'bootstrap' else 'golden.qcow2')
    expected = pins['cloud_sha256'] if role == 'bootstrap' else pins['golden_sha256']
    if source.is_symlink() or digest(source) != expected:
        raise ValueError('base image identity mismatch')
    launcher_sha256 = digest(Path(__file__))
    run_id = uuid.uuid4().hex
    # At most one VM per fixed role; the five ordinary jobs can run in parallel.
    lock = (ROOT / ('role-' + role + '.lock')).open('a')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    parent = ROOT / 'runs'
    parent.mkdir(mode=0o711, exist_ok=True)
    directory = parent / run_id
    directory.mkdir(mode=0o700)
    os.chown(directory, account.pw_uid, account.pw_gid)
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
        with (directory / 'qemu.log').open('wb') as log:
            process = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True)
            process.wait(timeout=timeout)
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
        success = True
    finally:
        if process is not None:
            stop_owned(process)
        # Preserve bounded diagnostics separately from executable work disks.
        report = {'id': run_id, 'role': role, 'started': started, 'finished': time.time(),
                  'vm_exited': process is not None and process.poll() is not None,
                  'provisioning_completed': success, 'base_sha256': expected,
                  'launcher_sha256': launcher_sha256,
                  'memory_mib': ROLES[role][0], 'cpus': ROLES[role][1]}
        (directory / 'report.json').write_text(json.dumps(report) + '\n')
        for name in ('user-data', 'meta-data', 'seed.img', 'work.qcow2'):
            path = directory / name
            if path.exists() and not path.is_symlink():
                path.unlink()
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
    args = parser.parse_args()
    if not 60 <= args.timeout <= 7200:
        parser.error('timeout must be between 60 and 7200 seconds')
    if args.jit_stdin != (args.role != 'bootstrap'):
        parser.error('only job VMs require --jit-stdin')
    try:
        pins = json.loads(args.pins.read_text())
        jit = sys.stdin.read(131073).strip() if args.jit_stdin else ''
        run(args.role, jit, pins, args.timeout)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print('CI VM provisioning failed: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
