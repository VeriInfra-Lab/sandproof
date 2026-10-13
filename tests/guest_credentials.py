#!/usr/bin/env python3
"""Explicit privileged worker component test, never an ordinary CI prerequisite.

Run a reviewed exact commit as PID 1 in a fresh mount/PID namespace with a
private proc mount. Requires --worker and its SHA-256. No host account changes,
VM, network configuration or broad process cleanup. This is not KVM acceptance.
"""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import socket
import select
import stat
import struct
import subprocess
import tempfile
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def read_exact(stream, count):
    data = bytearray()
    while len(data) != count:
        chunk = stream.recv(count - len(data))
        require(bool(chunk), 'truncated private worker reply')
        data.extend(chunk)
    return bytes(data)


def seal_worker(source, expected, directory):
    require(len(expected) == 64 and all(c in '0123456789abcdef' for c in expected),
            'explicit lowercase SHA-256 required')
    fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    destination = directory / 'worker'
    digest = hashlib.sha256()
    with os.fdopen(fd, 'rb') as incoming, destination.open('xb') as outgoing:
        info = os.fstat(incoming.fileno())
        require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= 128 * 1024 * 1024,
                'worker must be a bounded regular file')
        total = 0
        while chunk := incoming.read(128 * 1024):
            total += len(chunk)
            require(total <= 128 * 1024 * 1024, 'worker grew beyond copy budget')
            digest.update(chunk)
            outgoing.write(chunk)
        require(total == info.st_size and digest.hexdigest() == expected,
                'worker size or SHA-256 differs')
    destination.chmod(0o500)
    require(destination.stat().st_uid == 0, 'sealed worker must be root-owned')
    return destination


def run_worker(worker, directory, spec, *, namespace=False, early_error=False):
    gate_path = directory / 'gate'
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(str(gate_path))
        listener.listen(1)
        listener.settimeout(5)
        args = [str(worker), '--gate', str(gate_path), '--limit', '16384']
        if namespace:
            args = ['/usr/bin/unshare', '--user', '--map-root-user', '--'] + args
        master, slave = os.openpty() if spec.get('pty') is not None else (None, None)
        child = None
        try:
            child = subprocess.Popen(args, stdin=slave if slave is not None else subprocess.DEVNULL,
                                     stdout=slave if slave is not None else subprocess.PIPE,
                                     stderr=slave if slave is not None else subprocess.PIPE,
                                     env={'DAEMON_ONLY_SENTINEL': 'must-not-inherit'})
            if slave is not None:
                os.close(slave)
                slave = None
            with listener.accept()[0] as stream:
                stream.settimeout(5)
                require(read_exact(stream, 4) == b'SG01', 'invalid hello')
                payload = json.dumps(spec, separators=(',', ':')).encode()
                require(len(payload) <= 16384, 'fixture exceeded launch budget')
                stream.sendall(struct.pack('!I', len(payload)) + payload)
                ready = read_exact(stream, 4)
                if early_error:
                    require(ready == b'SGE1', 'malformed credentials reached ready')
                    diagnostic = ready + read_exact(stream, 4)
                else:
                    require(ready == b'SG02', 'worker did not prepare')
                    require(not (directory / 'effect').exists(), 'user code ran before release')
                    stream.sendall(b'\x01')
                    stream.shutdown(socket.SHUT_WR)
                    diagnostic = bytearray()
                    while chunk := stream.recv(8 - len(diagnostic)):
                        diagnostic.extend(chunk)
                        if len(diagnostic) == 8:
                            break
                    diagnostic = bytes(diagnostic)
            if master is None:
                stdout, stderr = child.communicate(timeout=5)
            else:
                stdout = read_terminal(master)
                stderr = b''
                child.wait(timeout=5)
            return child.returncode, diagnostic, stdout, stderr
        finally:
            if child is not None:
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
            for fd in (master, slave):
                if fd is not None:
                    os.close(fd)
            gate_path.unlink(missing_ok=True)


def read_terminal(master):
    os.set_blocking(master, False)
    until, data = time.monotonic() + 5, bytearray()
    while True:
        remaining = until - time.monotonic()
        require(remaining > 0 and select.select([master], [], [], remaining)[0],
                'terminal output deadline exceeded')
        try:
            chunk = os.read(master, 4096)
        except BlockingIOError:
            continue
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        data.extend(chunk)
        require(len(data) <= 16384, 'terminal output exceeded fixture budget')
    return bytes(data)


PROBE = r'''
import json, os, sys
from pathlib import Path
status = {}
for line in Path('/proc/self/status').read_text().splitlines():
    if line.startswith(('Uid:', 'Gid:', 'CapEff:')):
        key, value = line.split(':', 1)
        status[key] = value.split()
try:
    Path(sys.argv[1]).read_bytes()
    secret_denied = False
except PermissionError:
    secret_denied = True
try:
    os.setuid(0)
    root_denied = False
except PermissionError:
    root_denied = True
print(json.dumps(dict(uid=os.getuid(), gid=os.getgid(), groups=os.getgroups(),
    cwd=os.getcwd(), env=dict(os.environ), status=status,
    tty=[os.isatty(fd) for fd in range(3)], sid=os.getsid(0), pid=os.getpid(),
    secret_denied=secret_denied, root_denied=root_denied,
    group_file=Path(sys.argv[2]).read_text())))
'''


def check_positive(worker, directory, terminal):
    uid, gid, supplementary = 61001, 61002, 61003
    home = directory / 'home'
    home.mkdir(mode=0o750)
    os.chown(home, uid, gid)
    secret = directory / 'root-only'
    secret.write_text('synthetic root-only fixture')
    secret.chmod(0o600)
    group_file = directory / 'group-file'
    group_file.write_text('supplementary-group-readable')
    os.chown(group_file, 0, supplementary)
    group_file.chmod(0o640)
    env = dict(PATH='/usr/bin:/bin', LC_ALL='C.UTF-8', HOME='/pretend', USER='root')
    spec = dict(program='/usr/bin/python3', args=['-c', PROBE, str(secret), str(group_file)],
                cwd=str(home), env=env, stdin=False, pty=dict(cols=80, rows=24) if terminal else None,
                credentials=dict(uid=uid, gid=gid, groups=[gid, gid, supplementary]))
    code, diagnostic, stdout, stderr = run_worker(worker, directory, spec)
    require(code == 0 and not diagnostic and not stderr, 'credential probe did not finish cleanly')
    report = json.loads(stdout)
    require(report['tty'] == [terminal] * 3, 'terminal descriptor wiring differs')
    if terminal:
        require(report['sid'] == report['pid'], 'terminal worker did not own its session')
    require(report['uid'] == uid and report['gid'] == gid, 'wrong numeric identity')
    require(report['status']['Uid'] == [str(uid)] * 4, 'real/effective/saved/fs uid differs')
    require(report['status']['Gid'] == [str(gid)] * 4, 'real/effective/saved/fs gid differs')
    require(report['groups'] == [gid, gid, supplementary], 'supplementary groups differ')
    require(int(report['status']['CapEff'][0], 16) == 0, 'effective capabilities remained')
    require(report['secret_denied'] and report['root_denied'], 'root privilege was retained')
    require(report['env'] == env and report['cwd'] == str(home), 'execution context differs')
    require(report['group_file'] == 'supplementary-group-readable', 'group access was lost')
    return 1


def check_failures(worker, directory, terminal):
    groups = os.getgroups()
    cases = [
        (dict(uid=0, gid=0, groups=[61003]), True, False),  # setgroups denied
        (dict(uid=0, gid=61002, groups=groups), True, False),  # unmapped gid
        (dict(uid=61001, gid=0, groups=groups), True, False),  # unmapped uid
        (dict(uid=2**32-1, gid=0, groups=groups), False, True),  # sentinel
    ]
    marker = directory / 'effect'
    for credentials, namespace, early_error in cases:
        spec = dict(program='/usr/bin/python3',
                    args=['-c', 'import pathlib,sys; pathlib.Path(sys.argv[1]).write_text("ran")', str(marker)],
                    cwd=str(directory), env={}, stdin=False, credentials=credentials,
                    pty=dict(cols=80, rows=24) if terminal else None)
        code, diagnostic, stdout, stderr = run_worker(
            worker, directory, spec, namespace=namespace, early_error=early_error)
        require(code == 125 and len(diagnostic) == 8 and diagnostic[:4] == b'SGE1',
                'credential failure was not a private exec diagnostic')
        require(struct.unpack('!i', diagnostic[4:])[0] in (1, 22), 'unexpected credential errno')
        require(not stdout and not stderr and not marker.exists(), 'failure executed user code')
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--worker', required=True, type=Path)
    parser.add_argument('--sha256', required=True)
    args = parser.parse_args()
    require(os.geteuid() == 0 and os.getpid() == 1,
            'requires root as PID 1 in a fresh mount/PID namespace')
    status = Path('/proc/self/status').read_text()
    require(any(line.split() == ['NSpid:', '1'] for line in status.splitlines()),
            'requires a proc mount for the new PID namespace')
    require(args.worker.is_absolute(), 'absolute worker path required')
    with tempfile.TemporaryDirectory(prefix='sandproof-guest-credentials-') as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        worker = seal_worker(args.worker, args.sha256, directory)
        cases = 0
        for terminal in (False, True):
            case_directory = directory / ('pty' if terminal else 'pipes')
            case_directory.mkdir(mode=0o755)
            cases += check_positive(worker, case_directory, terminal)
            cases += check_failures(worker, case_directory, terminal)
            require(not (case_directory / 'gate').exists(), 'private socket remained')
        # All children are explicitly waited; PID namespace prevents accidental
        # references to host processes. Only this exact temporary tree is removed.
    print(json.dumps(dict(cases=cases, passed=True, worker_sha256=args.sha256,
                          scope='isolated Linux credential component; not KVM')))


if __name__ == '__main__':
    main()
