"""Shipping daemon restart checks in a fresh privileged PID/network namespace.

Run with the pinned SDK interpreter under unshare --mount --net --pid --fork
--mount-proc. This is a real daemon/process test with controlled MMDS, not a
Firecracker, host-gateway or template acceptance result. No production route,
metadata address or binary is changed to accommodate the fixture.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)['require_assertions']
_require_assertions()

import argparse
import asyncio
import ctypes
import hashlib
import http.client
import http.server
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import select
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time


TOKEN = 'isolated-daemon-restart-credential'
ADDRESS = '127.0.0.1:49983'
TASK_ROLES = {
    'asynchronous': ('connection', 'command_preparation', 'command_driver', 'command_registry'),
    'blocking': ('body_stream', 'body_encode', 'command_decode', 'file_decode',
                 'file_operation', 'file_upload', 'file_download', 'account_lookup',
                 'startup_open', 'daemon_initialize', 'daemon_load', 'pid_registration',
                 'command_preparation'),
}
HOLD = '''import os, pathlib, sys
root = pathlib.Path(sys.argv[1])
with (root / 'once').open('xb') as marker:
    marker.write(b'once')
fifo = os.open(root / 'release', os.O_RDWR)
os.write(1, b'ready')
assert os.read(fifo, 1) == b'x'
os.close(fifo)
'''


def require_namespace():
    # PID 1 is the probe itself, so namespace teardown is a final backstop for
    # unknown descendants. Refuse before configuring a link or binding MMDS.
    if os.geteuid() != 0 or os.getpid() != 1:
        raise RuntimeError('requires root as PID 1 in a new PID namespace')
    if [name for _, name in socket.if_nameindex()] != ['lo']:
        raise RuntimeError('requires a new network namespace with only loopback')
    if Path('/proc/1/exe').resolve() != Path('/proc/self/exe').resolve():
        raise RuntimeError('requires procfs mounted for the new PID namespace')
    if platform.machine() != 'x86_64':
        raise RuntimeError('this Linux ABI probe requires x86_64')


def pidfd_call(number, *arguments):
    # Linux x86_64 UAPI: pidfd_open=434, pidfd_send_signal=424. Managed
    # CPython builds may omit the os/signal wrappers even on a capable kernel.
    # No numeric-PID signal fallback is allowed.
    libc = ctypes.CDLL(None, use_errno=True)
    libc.syscall.restype = ctypes.c_long
    result = libc.syscall(ctypes.c_long(number), *arguments)
    if result < 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    return result


def fingerprint(path):
    digest = hashlib.sha256()
    size = 0
    with Path(path).open('rb') as source:
        for block in iter(lambda: source.read(65536), b''):
            size += len(block)
            digest.update(block)
    return dict(bytes=size, sha256=digest.hexdigest())


def task_report(output, prefix=b''):
    """Validate a bounded, exact, settled observation; never infer VM cleanup."""
    assert len(output) <= 4096 and output.startswith(prefix + b'task accounting: ')
    payload = output[len(prefix + b'task accounting: '):]
    assert payload.endswith(b'\n') and payload.count(b'\n') == 1

    def unique(pairs):
        result = {}
        for key, value in pairs:
            assert key not in result
            result[key] = value
        return result

    report = json.loads(payload, object_pairs_hook=unique)
    assert set(report) == {'schema_version', 'scope', 'exact', 'total', 'asynchronous', 'blocking'}
    assert type(report['schema_version']) is int and report['schema_version'] == 1
    assert report['scope'] == 'process_service_owned_tasks' and report['exact'] is True

    def count(value):
        assert set(value) == {'live', 'peak'}
        assert all(type(n) is int and 0 <= n <= (1 << 64) - 1 for n in value.values())
        assert value['live'] == 0 and value['peak'] >= value['live']
        return value['peak']

    total_peak = count(report['total'])
    peaks = []
    for name, expected in TASK_ROLES.items():
        group = report[name]
        assert set(group) == {'count', 'roles'}
        peak = count(group['count'])
        assert type(group['roles']) is list and len(group['roles']) == len(expected)
        role_peaks = []
        for role, expected_name in zip(group['roles'], expected):
            assert set(role) == {'name', 'count'} and role['name'] == expected_name
            role_peaks.append(count(role['count']))
        assert max(role_peaks) <= peak <= sum(role_peaks)
        peaks.append(peak)
    assert max(peaks) <= total_peak <= sum(peaks)
    return report


class Metadata(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def reply(self, body, content_type):
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(body)

    def do_PUT(self):
        assert self.path == '/latest/api/token'
        assert self.headers['x-metadata-token-ttl-seconds'] == '60'
        self.reply(b'fixture-mmds-session', 'text/plain')

    def do_GET(self):
        assert self.path == '/' and self.headers['x-metadata-token'] == 'fixture-mmds-session'
        self.server.observations += 1
        self.reply(json.dumps(self.server.metadata).encode(), 'application/json')

    def log_message(self, *_args):
        pass


def exchange(method, path, body=None):
    connection = http.client.HTTPConnection(ADDRESS, timeout=2)
    try:
        connection.request(method, path, body=json.dumps(body) if body is not None else None,
                           headers={'Content-Type': 'application/json'})
        response = connection.getresponse()
        payload = response.read(4097)
        assert len(payload) <= 4096
        return response.status, payload
    finally:
        connection.close()


class Process:
    def __init__(self, pid):
        self.pid = pid
        self.fd = pidfd_call(434, ctypes.c_int(pid), ctypes.c_uint(0))
        self.start = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]

    def alive(self):
        assert not select.select([self.fd], [], [], 0)[0], 'original process exited'
        assert Path(f'/proc/{self.pid}/stat').read_text().rsplit(')', 1)[1].split()[19] == self.start

    def wait(self):
        assert select.select([self.fd], [], [], 10)[0], 'original process did not exit'
        return os.waitid(3, self.fd, os.WEXITED)  # Linux P_PIDFD from linux/wait.h.

    def kill(self):
        if not select.select([self.fd], [], [], 0)[0]:
            pidfd_call(424, ctypes.c_int(self.fd), ctypes.c_int(signal.SIGKILL),
                       ctypes.c_void_p(), ctypes.c_uint(0))

    def close(self):
        os.close(self.fd)


class Daemon:
    def __init__(self, binary, config):
        self.child = subprocess.Popen([str(binary), 'serve', '--config', str(config)],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env={'PATH': '/usr/bin:/bin'})
        self.process = Process(self.child.pid)

    def ready(self):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            assert self.child.poll() is None, 'daemon exited before listening'
            try:
                assert exchange('GET', '/health') == (204, b'')
                return
            except (ConnectionRefusedError, ConnectionResetError, TimeoutError):
                # Only retry a side-effect-free readiness observation. Never
                # replay initialize, Start, input, a signal or a file write.
                time.sleep(0.01)
        raise TimeoutError('daemon did not bind within readiness deadline')

    def stop(self):
        self.process.kill()
        output = self.child.communicate(timeout=10)
        assert self.child.returncode == -signal.SIGKILL
        assert output == (b'', b''), 'unexpected formal daemon output'

    def finish(self):
        pidfd_call(424, ctypes.c_int(self.process.fd), ctypes.c_int(signal.SIGTERM),
                   ctypes.c_void_p(), ctypes.c_uint(0))
        stdout, stderr = self.child.communicate(timeout=40)
        assert self.child.returncode == 0 and not stderr
        lines = stdout.splitlines(keepends=True)
        assert len(lines) == 2 and lines[0].startswith(b'transport stopped: Report {')
        return task_report(lines[1])

    def close(self):
        try:
            if self.child.poll() is None:
                self.process.kill()
            self.child.communicate(timeout=10)
        finally:
            self.process.close()


class Peer:
    def __init__(self, asynchronous):
        from e2b.connection_config import ConnectionConfig
        from e2b.envd.client_async import create_rpc_client as async_client
        from e2b.envd.client_sync import create_rpc_client as sync_client
        from e2b.envd.process import process_connect
        self.asynchronous = asynchronous
        factory, kind = ((async_client, process_connect.ProcessClient) if asynchronous
                         else (sync_client, process_connect.ProcessClientSync))
        self.client = factory(kind, 'http://' + ADDRESS, ConnectionConfig())

    def call(self, method, request):
        return getattr(self.client, method)(request, headers={'x-access-token': TOKEN}, timeout_ms=10000)

    async def unary(self, method, request):
        result = self.call(method, request)
        return await result if self.asynchronous else result

    async def next(self, stream):
        if self.asynchronous:
            return await anext(stream)
        try:
            return next(stream)
        except StopIteration:
            raise StopAsyncIteration from None

    async def close(self, stream):
        if self.asynchronous:
            await stream.aclose()
        else:
            stream.close()

    async def rejected(self, method, kind, payload, code, message=None):
        from connectrpc.errors import ConnectError
        try:
            request = kind.from_json(json.dumps(payload))
            if method == 'connect':
                stream = self.call(method, request)
                try:
                    await self.next(stream)
                finally:
                    await self.close(stream)
            else:
                await self.unary(method, request)
        except ConnectError as error:
            assert error.code == code, (method, error.code)
            if message is not None:
                assert error.message == message, (method, error.message)
        else:
            raise AssertionError(method + ' accepted a rejected request')


async def case(binary, worker, template, metadata, asynchronous):
    from connectrpc.code import Code
    from e2b.envd.process import process_pb as pb
    with tempfile.TemporaryDirectory(prefix='sandproof-daemon-restart-', dir='/root') as temporary:
        root = Path(temporary)
        for directory in ('state', 'gates'):
            (root / directory).mkdir(mode=0o700)
        installed = root / 'worker'
        shutil.copyfile(worker, installed)
        installed.chmod(0o500)
        version = json.loads(subprocess.check_output([str(binary), '--version'], timeout=5))
        config = json.loads(template.read_text())
        config['version'] = version['config_version']
        config['listen'] = ADDRESS
        config['paths'].update(owner_uid=0, pid_directory=str(root / 'state'),
                               gate_directory=str(root / 'gates'), worker=str(installed))
        config['worker'] = dict(package_version=version['worker_package_version'],
                                protocol_version=version['worker_protocol_version'],
                                artifact=fingerprint(installed))
        path = root / 'configuration.json'
        path.write_text(json.dumps(config))
        path.chmod(0o600)
        receipt = dict(asynchronous=asynchronous, configuration=fingerprint(path), checks=[])
        provision = subprocess.run([str(binary), 'initialize', '--config', str(path)],
                                   capture_output=True, timeout=40, check=True)
        assert not provision.stderr
        receipt['tasks_initialize'] = task_report(provision.stdout, b'new VM state initialized\n')
        initial = receipt['tasks_initialize']
        assert initial['asynchronous']['count']['peak'] == 0
        assert all(role['count']['peak'] == int(role['name'] in {'daemon_load', 'daemon_initialize'})
                   for role in initial['blocking']['roles'])
        claims = root / 'state/pid-claims.bin'
        claim_inode = claims.stat().st_ino
        daemons, original, stream = [], None, None
        peer = Peer(asynchronous)
        try:
            first = Daemon(binary, path)
            daemons.append(first)
            first.ready()
            await peer.rejected('list', pb.ListRequest, {}, Code.UNAUTHENTICATED)
            assert exchange('POST', '/init', {'accessToken': 'wrong'})[0] == 401
            assert exchange('POST', '/init', {'accessToken': TOKEN}) == (204, b'')
            os.mkfifo(root / 'release', 0o600)
            request = pb.StartRequest.from_json(json.dumps(dict(
                process=dict(cmd='/usr/bin/python3', args=['-I', '-c', HOLD, str(root)]),
                stdin=True, tag='old-service-tag')))
            stream = peer.call('start', request)
            event = (await peer.next(stream)).event.event
            assert event.field == 'start'
            original = Process(event.value.pid)
            ready = bytearray()
            while len(ready) < 5:
                event = (await peer.next(stream)).event.event
                if event.field == 'keepalive':
                    continue
                assert event.field == 'data' and event.value.output.field == 'stdout'
                ready.extend(event.value.output.value)
            assert ready == b'ready'
            assert (root / 'once').read_bytes() == b'once'
            original.alive()
            # The live subscription must not receive a fabricated End when
            # the actual daemon dies. The SDK sees an incomplete stream.
            first.stop()
            try:
                while True:
                    event = (await peer.next(stream)).event.event
                    assert event.field != 'end', 'daemon death fabricated command completion'
            except (StopIteration, StopAsyncIteration):
                pass
            except Exception as error:
                # A transport failure is allowed, an assertion is not.
                from connectrpc.errors import ConnectError
                assert isinstance(error, (ConnectError, http.client.HTTPException)), type(error).__name__
            finally:
                await peer.close(stream)
                stream = None
            original.alive()
            receipt['checks'].append('daemon-killed-original-command-alive-no-end')
            second = Daemon(binary, path)
            daemons.append(second)
            second.ready()
            assert claims.stat().st_ino == claim_inode
            await peer.rejected('list', pb.ListRequest, {}, Code.UNAUTHENTICATED)
            assert exchange('POST', '/init', {'accessToken': 'wrong'})[0] == 401
            assert exchange('POST', '/init', {'accessToken': TOKEN}) == (204, b'')
            assert (await peer.unary('list', pb.ListRequest())).processes == []
            for selector in ({'pid': original.pid}, {'tag': 'old-service-tag'}):
                field, value = next(iter(selector.items()))
                message = f'process with {field} {value} not found'
                for method, kind, extra in [
                    ('connect', pb.ConnectRequest, {}),
                    ('send_input', pb.SendInputRequest, {'input': {'stdin': 'eA=='}}),
                    ('close_stdin', pb.CloseStdinRequest, {}),
                    ('send_signal', pb.SendSignalRequest, {'signal': 'SIGNAL_SIGKILL'}),
                    ('update', pb.UpdateRequest, {'pty': {'size': {'rows': 30, 'cols': 80}}}),
                ]:
                    await peer.rejected(method, kind, {'process': selector} | extra, Code.NOT_FOUND, message)
                    original.alive()
            receipt['checks'].append('restart-requires-auth-and-refuses-all-old-pid-and-tag-operations')
            assert (root / 'once').read_bytes() == b'once'
            # Rejecting every command is not recovery. A fresh command with
            # the same tag succeeds while the unadopted original is still live.
            fresh = pb.StartRequest.from_json(json.dumps(dict(
                process=dict(cmd='/usr/bin/python3', args=['-I', '-c',
                    "import pathlib,sys; pathlib.Path(sys.argv[1]).open('xb').write(b'new'); print('new')",
                    str(root / 'new-once')]), tag='old-service-tag')))
            stream = peer.call('start', fresh)
            event = (await peer.next(stream)).event.event
            assert event.field == 'start' and event.value.pid != original.pid
            output = bytearray()
            ends = 0
            while True:
                try:
                    event = (await peer.next(stream)).event.event
                except StopAsyncIteration:
                    break
                if event.field == 'keepalive':
                    continue
                if event.field == 'end':
                    assert ends == 0 and event.value.exited and event.value.exit_code == 0
                    assert event.value.status == 'exit status 0' and not event.value.has_field('error')
                    ends += 1
                else:
                    assert ends == 0 and event.field == 'data' and event.value.output.field == 'stdout'
                    output.extend(event.value.output.value)
                    assert len(output) <= 4
            await peer.close(stream)
            stream = None
            assert ends == 1 and output == b'new\n'
            assert (root / 'new-once').read_bytes() == b'new'
            original.alive()
            receipt['checks'].append('fresh-command-and-tag-work-without-taking-over-old-process')
            # Release the exact original FIFO independently of the restarted
            # registry; PID 1 adopts and waits the original orphan via pidfd.
            with os.fdopen(os.open(root / 'release', os.O_WRONLY | os.O_NONBLOCK), 'wb', buffering=0) as release:
                release.write(b'x')
            result = original.wait()
            assert result.si_code == os.CLD_EXITED and result.si_status == 0
            receipt['checks'].append('original-command-exited-once-without-adoption-or-replay')
            receipt['tasks_serve'] = second.finish()
            observed = receipt['tasks_serve']
            assert all(role['count']['peak'] > 0 for role in observed['asynchronous']['roles'])
            required = {'command_decode', 'account_lookup', 'startup_open', 'daemon_load',
                        'pid_registration', 'command_preparation'}
            assert all(role['count']['peak'] > 0 for role in observed['blocking']['roles']
                       if role['name'] in required)
            receipt['checks'].append('graceful-shutdown-reports-settled-owned-task-peaks')
            frozen = claims.read_bytes()
            metadata.metadata['instanceID'] = 'different-vm'
            rejected = subprocess.run([str(binary), 'serve', '--config', str(path)],
                                      capture_output=True, timeout=40)
            assert rejected.returncode == 1
            receipt['tasks_refused_startup'] = task_report(rejected.stdout)
            refused = receipt['tasks_refused_startup']
            assert refused['asynchronous']['count']['peak'] == 0
            assert all(role['count']['peak'] == int(role['name'] in {'daemon_load', 'startup_open'})
                       for role in refused['blocking']['roles'])
            assert rejected.stderr == b'guest service failed: Service(Startup(Storage(BindingMismatch)))\n'
            assert claims.read_bytes() == frozen and claims.stat().st_ino == claim_inode
            receipt['checks'].append('different-vm-refused-with-original-state-unchanged')
            return receipt
        finally:
            metadata.metadata['instanceID'] = 'same-vm'
            errors = []
            if stream is not None:
                try:
                    await peer.close(stream)
                except BaseException as error:
                    errors.append(error)
            for daemon in reversed(daemons):
                try:
                    daemon.close()
                except BaseException as error:
                    errors.append(error)
            if original is not None:
                try:
                    original.kill()
                    try:
                        original.wait()
                    except ChildProcessError:
                        pass  # Already reaped through this same pidfd above.
                except BaseException as error:
                    errors.append(error)
                finally:
                    original.close()
            if errors:
                raise BaseExceptionGroup('daemon restart fixture cleanup failed', errors)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--service', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--configuration', type=Path, required=True)
    args = parser.parse_args()
    require_namespace()
    assert importlib.metadata.version('e2b') == '2.51.0'
    for key in tuple(os.environ):
        if key.startswith('E2B_') or key.lower() in {'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy'}:
            del os.environ[key]
    subprocess.run(['ip', 'link', 'set', 'lo', 'up'], check=True, timeout=5)
    subprocess.run(['ip', 'addr', 'add', '169.254.169.254/32', 'dev', 'lo'], check=True, timeout=5)
    with http.server.HTTPServer(('169.254.169.254', 80), Metadata) as server:
        server.metadata = dict(instanceID='same-vm', envID='same-template',
                               accessTokenHash=hashlib.sha512(TOKEN.encode()).hexdigest())
        server.observations = 0
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            receipts = [asyncio.run(case(args.service.resolve(), args.worker.resolve(),
                                         args.configuration.resolve(), server, mode))
                        for mode in (False, True)]
        finally:
            server.shutdown()
            thread.join(timeout=5)
        assert not thread.is_alive()
        print(json.dumps(dict(scope='shipping daemon; controlled MMDS; no KVM or gateway',
                              service=fingerprint(args.service), worker=fingerprint(args.worker),
                              probe=fingerprint(__file__), metadata_reads=server.observations,
                              receipts=receipts), sort_keys=True))


if __name__ == '__main__':
    main()
