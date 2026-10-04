"""Unshortened request/write idle deadlines on an externally supervised service.

The operator supplies two fresh, long-lived VMs, a fixed controller/config and
its private service cgroup record. This client never creates or destroys VMs.
Raw loopback HTTP controls body progress and TCP backpressure; the pinned SDK
checks admission, reserved control, and an independent tenant. Host reclamation
belongs to the outer supervisor. No timeout override is accepted.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
from datetime import datetime, timezone
import hashlib
from http.client import HTTPResponse, IncompleteRead
import importlib.metadata
import json
import os
from pathlib import Path
import re
import shlex
import socket
import struct
import subprocess
import sys
import time
from urllib.parse import urlencode, urlsplit
import uuid

IDLE_SECONDS = 13000
RESUME_SECONDS = IDLE_SECONDS // 2
FINISH_SECONDS = IDLE_SECONDS + 60
DEADLINE_MARGIN = 180
MIB = 1024 * 1024
DOWNLOAD_BYTES = 64 * MIB
PROGRESS_BYTES = 8 * MIB


def received_bytes(sock):
    """Linux TCP_INFO bytes_received, including bytes still queued at receiver."""
    info = sock.getsockopt(socket.IPPROTO_TCP, socket.TCP_INFO, 256)
    if len(info) < 136:
        raise ValueError('Linux TCP_INFO with tcpi_bytes_received is required')
    return struct.unpack_from('=Q', info, 128)[0]


def peer_snapshot(sock, server_pid):
    """Look up only this loopback connection and ownership by the fixed server."""
    client, server = sock.getsockname(), sock.getpeername()
    if client[0] != '127.0.0.1' or server[0] != '127.0.0.1':
        raise ValueError('explicit IPv4 loopback TCP connection required')
    local, remote = f'0100007F:{server[1]:04X}', f'0100007F:{client[1]:04X}'
    rows = [row.split() for row in Path(f'/proc/{server_pid}/net/tcp').read_text().splitlines()[1:]]
    matches = [row for row in rows if row[1:3] == [local, remote]]
    if len(matches) > 1:
        raise AssertionError('ambiguous exact TCP tuple')
    if not matches:
        return dict(owned=False, received_bytes=received_bytes(sock), state=None, tx_queue=0)
    row = matches[0]
    owned = False
    for fd in Path(f'/proc/{server_pid}/fd').iterdir():
        try:
            owned |= os.readlink(fd) == f'socket:[{row[9]}]'
        except FileNotFoundError:
            continue
    return dict(owned=owned, received_bytes=received_bytes(sock), state=row[3],
                tx_queue=int(row[4].split(':')[0], 16))


def assert_metadata(result):
    body = json.loads(result['body'])
    size = body.get('entry', {}).get('size')
    if result['status'] != 200 or str(size) != str(DOWNLOAD_BYTES):
        raise AssertionError('metadata operation did not return the exact fixture size')


def assert_expiry(elapsed, status):
    if status != 408 or not IDLE_SECONDS <= elapsed <= IDLE_SECONDS + DEADLINE_MARGIN:
        raise AssertionError('request did not expire with HTTP 408 at the unchanged idle deadline')


class Transfer:
    def __init__(self, url, token, *, small_window=False):
        if not re.fullmatch(r'[0-9a-fA-F]+', token):
            raise ValueError('hex envd token required')
        address = urlsplit(url)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            if small_window:
                # Set before connect, disabling receive-buffer autotuning.
                self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
            self.sock.settimeout(30)
            self.sock.connect((address.hostname, address.port))
        except BaseException:
            self.sock.close()
            raise
        self.token = token
        self.response = None
        self.started = time.monotonic()
        self.total = 0
        self.sha = hashlib.sha256()
        self.closed = False

    def head(self, method, route, extra=''):
        self.started = time.monotonic()
        self.sock.sendall((f'{method} {route} HTTP/1.1\r\nHost: localhost\r\n'
                           f'X-Access-Token: {self.token}\r\nConnection: close\r\n'
                           f'{extra}\r\n').encode())

    def chunk(self, data):
        if not data:
            raise ValueError('progress must be a nonempty data chunk')
        self.sock.sendall(f'{len(data):x}\r\n'.encode() + data + b'\r\n')

    def metadata(self):
        self.response = HTTPResponse(self.sock)
        self.response.begin()
        elapsed = time.monotonic() - self.started
        body = self.response.read(MIB + 1)
        if len(body) > MIB:
            raise AssertionError('metadata response exceeds RPC limit')
        return dict(status=self.response.status, elapsed_seconds=elapsed, body=body)

    def start_download(self, path):
        self.head('GET', '/files?' + urlencode(dict(path=path)))
        self.response = HTTPResponse(self.sock)
        self.response.begin()
        if self.response.status != 200:
            raise AssertionError('download did not start with HTTP 200')
        if not self.response.chunked and self.response.length != DOWNLOAD_BYTES:
            raise AssertionError('download must have verifiable HTTP message framing')
        self.take(1024)

    def account(self, data):
        if not data or data != b'X' * len(data):
            raise AssertionError('empty or corrupt file data')
        self.total += len(data)
        self.sha.update(data)
        if self.total > DOWNLOAD_BYTES:
            raise AssertionError('download exceeds the fixture length')

    def take(self, count):
        remaining = count
        while remaining:
            data = self.response.read(min(65536, remaining))
            self.account(data)
            remaining -= len(data)

    def finish(self, *, truncated):
        # A client timeout, connection reset, or arbitrary OSError is NOT proof
        # of the expected incomplete HTTP message. Require framing/length loss.
        incomplete = False
        try:
            while True:
                data = self.response.read(65536)
                if not data:
                    incomplete = self.response.length not in (None, 0)
                    break
                self.account(data)
        except IncompleteRead as error:
            if error.partial:
                self.account(error.partial)
            incomplete = True
        if truncated:
            if not incomplete or not 0 < self.total < DOWNLOAD_BYTES:
                raise AssertionError('expired writer did not produce an incomplete HTTP body')
        elif incomplete or self.total != DOWNLOAD_BYTES:
            raise AssertionError('progressing download was truncated')
        return dict(bytes=self.total, sha256=self.sha.hexdigest(), incomplete_http_body=incomplete,
                    elapsed_seconds=time.monotonic() - self.started)

    def close(self):
        if self.closed:
            return
        self.closed = True
        # Interrupt any pending to_thread response reader before joining it.
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        if self.response is not None:
            self.response.close()
        self.sock.close()


async def admission(target, control, directory, url, held, helpers):
    tracked = []
    try:
        for index in range(7 - held):
            await helpers.start_input(target, f'{directory}/barrier-{held}-{index}', tracked)
        await helpers.limited(target.files.get_info, directory, request_timeout=10)
        await helpers.health(target, url)
        await helpers.healthy(control, directory + '/control')
        for item in tracked:
            await helpers.end_input(item, b'reserved control works')
        await helpers.wait_capacity(target, directory)
        for item in tracked:
            if await helpers.call(target.files.read, item['marker']) != 'once\n':
                raise AssertionError('a command Start was replayed')
        return dict(held_transfers=held, barrier_commands=len(tracked), extra_data_429=True,
                    reserved_health_input_eof=True, starts_exactly_once=True)
    finally:
        await helpers.cleanup_handles(tracked)


async def backpressure(transfer, pid, samples):
    deadline = time.monotonic() + 20
    previous = None
    stable = 0
    while True:
        row = peer_snapshot(transfer.sock, pid)
        row['elapsed_seconds'] = time.monotonic() - transfer.started
        samples.append(row)
        if not row['owned'] or row['state'] != '01':
            raise AssertionError('server socket is not an established writer')
        stable = stable + 1 if row['tx_queue'] > 0 and previous == row['received_bytes'] else 0
        if stable >= 3:
            return row
        if time.monotonic() >= deadline:
            raise AssertionError('receiver did not reach stable TCP backpressure')
        previous = row['received_bytes']
        await asyncio.sleep(1)


def polling_delay(now, write_started, *, expired):
    """Wake at the narrow observation window instead of stepping over it."""
    if expired:
        return 30
    until_window = write_started + IDLE_SECONDS - 10 - now
    return min(30, until_window) if until_window > 0 else 1


async def exercise(args, report, checkpoint=lambda _report: None):
    from e2b import Sandbox
    import gateway_pressure as helpers
    from gateway_slow_boundaries import (digest, loopback_url, private_json,
        process_binding, start_ticks, validate_configuration)
    from sdk_server_smoke import options
    from tools import service_cgroup

    url = loopback_url(args.sandbox_url)
    settings = private_json(args.config)
    report['data_transfer'] = validate_configuration(settings)
    if loopback_url(args.api_url) != url or settings['bind'] != url.removeprefix('http://'):
        raise ValueError('fixed server and client origins must match')
    report['config_sha256'] = digest(args.config)
    report['server'] = process_binding(args.server_pid, args.server_start_ticks, args.config)
    report['sdk_version'] = importlib.metadata.version('e2b')
    if report['sdk_version'] != '2.51.0':
        raise ValueError('locked original SDK 2.51.0 required')
    baseline = service_cgroup.observe(args.service_cgroup_record, pids=[args.server_pid])
    report.update(service_baseline=baseline, service_samples=[], cases={}, phase='preparing',
                  host_cleanup='outer_supervisor_required')
    keys = [os.environ.get(name) for name in ('SANDPROOF_TEST_API_KEY', 'SANDPROOF_TEST_FOREIGN_API_KEY')]
    if not all(keys) or keys[0] == keys[1] or args.target_sandbox_id == args.control_sandbox_id:
        raise ValueError('two distinct explicit tenants and VMs required')
    instances = []
    for ident, key in zip((args.target_sandbox_id, args.control_sandbox_id), keys):
        instance = await helpers.call(Sandbox.connect, ident, **(options(key, url, url) | dict(request_timeout=120, retries=0)))
        info = await helpers.call(instance.get_info)
        remaining = (info.end_at - datetime.now(timezone.utc)).total_seconds()
        if instance.sandbox_id != ident or info.state != 'running' or remaining < FINISH_SECONDS + 600:
            raise ValueError('operator-owned running VM lifetime is too short for unchanged deadlines')
        report.setdefault('initial_remaining_seconds', []).append(remaining)
        instances.append(instance)
    target, control = instances
    report['sandbox_ids'] = [instance.sandbox_id for instance in instances]
    directory = '/tmp/gateway-long-idle-' + uuid.uuid4().hex
    created, transfers, readers = [], [], []
    report['guest_directory'] = directory
    try:
        for instance in instances:
            result = await helpers.call(instance.commands.run, 'mkdir -- ' + shlex.quote(directory), timeout=15)
            if result.exit_code != 0:
                raise AssertionError('guest scratch directory creation failed')
            created.append(instance)
        path = directory + '/large.bin'
        result = await helpers.call(target.commands.run,
            f"head -c {DOWNLOAD_BYTES} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30)
        if result.exit_code != 0 or (await helpers.call(target.files.get_info, path)).size != DOWNLOAD_BYTES:
            raise AssertionError('64 MiB download fixture is incomplete')
        # Prove this exact JSON route/auth/response schema before the long wait.
        preflight = Transfer(url, target._envd_access_token)
        try:
            preflight.head('POST', '/filesystem.Filesystem/Stat',
                'Content-Type: application/json\r\nConnect-Protocol-Version: 1\r\nTransfer-Encoding: chunked\r\n')
            preflight.chunk(json.dumps(dict(path=path)).encode())
            preflight.sock.sendall(b'0\r\n\r\n')
            assert_metadata(await asyncio.to_thread(preflight.metadata))
        finally:
            preflight.close()
        for _ in range(4):
            transfers.append(Transfer(url, target._envd_access_token, small_window=True))
        request_expiry, request_progress, write_expiry, write_progress = transfers
        for transfer in (request_expiry, request_progress):
            transfer.head('POST', '/filesystem.Filesystem/Stat',
                'Content-Type: application/json\r\nConnect-Protocol-Version: 1\r\nTransfer-Encoding: chunked\r\n')
            transfer.chunk(b' ')
            transfer.sock.settimeout(IDLE_SECONDS + DEADLINE_MARGIN)
            readers.append(asyncio.create_task(asyncio.to_thread(transfer.metadata)))
        for transfer in (write_expiry, write_progress):
            await asyncio.to_thread(transfer.start_download, path)
        samples = report['tcp_samples'] = {'expiry': [], 'progress': []}
        for name, transfer in (('expiry', write_expiry), ('progress', write_progress)):
            report['cases']['write_' + name] = dict(initial_backpressure=await backpressure(transfer, args.server_pid, samples[name]))
        report['initial_admission'] = await admission(target, control, directory, url, 4, helpers)
        started = min(transfer.started for transfer in transfers)
        deadline = max(transfer.started for transfer in transfers) + FINISH_SECONDS
        reset_at = started + RESUME_SECONDS
        reset = False
        expired_at = None
        last_owned = 0
        report.update(phase='holding', observation_started_utc=datetime.now(timezone.utc).isoformat())
        while time.monotonic() < deadline:
            now = time.monotonic()
            if start_ticks(args.server_pid) != args.server_start_ticks:
                raise AssertionError('original controller exited or was replaced')
            observation = service_cgroup.observe(args.service_cgroup_record, pids=[args.server_pid])
            observation['elapsed_seconds'] = now - started
            report['service_samples'].append(observation)
            if observation['identity'] != baseline['identity'] or observation['limits'] != baseline['limits']:
                raise AssertionError('service cgroup identity or hard limits changed')
            if any(observation['memory_events'].get(key, 0) != baseline['memory_events'].get(key, 0)
                   for key in ('oom', 'oom_kill', 'oom_group_kill')):
                raise AssertionError('service OOM counter increased')
            for name, transfer in (('expiry', write_expiry), ('progress', write_progress)):
                row = peer_snapshot(transfer.sock, args.server_pid)
                row['elapsed_seconds'] = time.monotonic() - transfer.started
                samples[name].append(row)
                if name == 'progress' and not row['owned']:
                    raise AssertionError('progressing writer closed prematurely')
                if name == 'expiry':
                    if row['owned']:
                        last_owned = row['elapsed_seconds']
                    elif expired_at is None:
                        expired_at = row['elapsed_seconds']
                        if not IDLE_SECONDS - 5 <= last_owned <= expired_at <= IDLE_SECONDS + DEADLINE_MARGIN:
                            raise AssertionError('writer expired outside the observed unchanged deadline interval')
                        report['cases']['write_expiry']['closure_interval_seconds'] = [last_owned, expired_at]
            if readers[1].done():
                raise AssertionError('unfinished progressing request returned prematurely')
            if readers[0].done():
                result = readers[0].result()
                assert_expiry(result['elapsed_seconds'], result['status'])
                report['cases']['request_expiry'] = {key: result[key] for key in ('status', 'elapsed_seconds')}
            if not reset and now >= reset_at:
                request_progress.chunk(b' ')
                report['request_progress_at_seconds'] = time.monotonic() - request_progress.started
                before = received_bytes(write_progress.sock)
                await asyncio.to_thread(write_progress.take, PROGRESS_BYTES)
                after = await backpressure(write_progress, args.server_pid, samples['progress'])
                if after['received_bytes'] - before < PROGRESS_BYTES // 2:
                    raise AssertionError('resume did not produce new TCP receive progress')
                report['write_progress_at_seconds'] = time.monotonic() - write_progress.started
                report['write_progress_received_delta'] = after['received_bytes'] - before
                reset = True
                report['phase'] = 'holding_after_progress'
            await helpers.health(target, url)
            await helpers.health(control, url)
            await helpers.healthy(control, directory + '/control')
            report['elapsed_seconds'] = time.monotonic() - started
            checkpoint(report)
            # Near expiry preserve a <=5s observed closure interval; elsewhere
            # cadence is only resource sampling, never a race/admission barrier.
            delay = polling_delay(time.monotonic(), write_expiry.started, expired=expired_at is not None)
            await asyncio.sleep(min(delay, max(0, deadline - time.monotonic())))
        if not reset or expired_at is None or not readers[0].done():
            raise AssertionError('unchanged idle expiry was not observed')
        report['phase'] = 'finishing'
        report['cases']['write_expiry'].update(await asyncio.to_thread(write_expiry.finish, truncated=True))
        # Both expired requests must release their ordinary admission leases.
        report['after_expiry_admission'] = await admission(target, control, directory, url, 2, helpers)
        request_progress.chunk(json.dumps(dict(path=path)).encode())
        request_progress.sock.sendall(b'0\r\n\r\n')
        result = await asyncio.wait_for(readers[1], timeout=30)
        assert_metadata(result)
        report['cases']['request_progress'] = {key: result[key] for key in ('status', 'elapsed_seconds')}
        report['cases']['write_progress'].update(await asyncio.to_thread(write_progress.finish, truncated=False))
        expected = hashlib.sha256()
        for _ in range(DOWNLOAD_BYTES // 65536):
            expected.update(b'X' * 65536)
        if report['cases']['write_progress']['sha256'] != expected.hexdigest():
            raise AssertionError('resumed full download digest differs from fixture')
        for transfer in transfers:
            transfer.close()
        report['final_admission'] = await admission(target, control, directory, url, 0, helpers)
        if digest(args.config) != report['config_sha256'] or process_binding(args.server_pid, args.server_start_ticks, args.config) != report['server']:
            raise AssertionError('original server/configuration binding changed')
        report['phase'] = 'passed'
    finally:
        for transfer in transfers:
            transfer.close()
        if readers:
            await asyncio.gather(*readers, return_exceptions=True)
        errors = []
        names = ['large.bin', 'control'] + [f'barrier-{held}-{index}' for held in (4, 2, 0) for index in range(7 - held)]
        for instance in created:
            try:
                exact = ' '.join(shlex.quote(directory + '/' + name) for name in names)
                result = await helpers.call(instance.commands.run, f'rm -f -- {exact}; rmdir -- {shlex.quote(directory)}', timeout=30)
                if result.exit_code != 0:
                    raise AssertionError('exact guest fixture cleanup failed')
            except Exception as error:
                errors.append(type(error).__name__)
        report['client_cleanup_errors'] = errors
        checkpoint(report)
        if errors:
            raise AssertionError('outer supervisor must reclaim the exact original VMs')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('api-url', 'sandbox-url', 'target-sandbox-id', 'control-sandbox-id'):
        parser.add_argument('--' + field, required=True)
    for field in ('config', 'service-cgroup-record', 'report'):
        parser.add_argument('--' + field, type=Path, required=True)
    for field in ('server-pid', 'server-start-ticks'):
        parser.add_argument('--' + field, type=int, required=True)
    args = parser.parse_args(argv)
    if os.geteuid() != 0 or sys.platform != 'linux':
        parser.error('root Linux with an outer approved supervisor required')
    from gateway_slow_boundaries import digest
    from tools import service_cgroup
    service_cgroup.private(args.report.parent)
    if not args.report.is_absolute() or args.report.is_symlink():
        parser.error('new absolute private report path required')
    fd = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    report = dict(schema_version=1, result='incomplete', host_cleanup='outer_supervisor_required')
    code = 1
    root = Path(__file__).resolve().parents[2]
    try:
        if subprocess.check_output(['git', 'status', '--porcelain'], cwd=root).strip():
            raise ValueError('clean reviewed source checkout required')
        report['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
        report['probe_sha256'] = digest(__file__)
        asyncio.run(exercise(args, report))
        report['result'], code = 'passed', 0
    except Exception as error:
        report['failure_type'] = type(error).__name__
    finally:
        with os.fdopen(fd, 'w') as output:
            json.dump(report, output, sort_keys=True)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
    print(json.dumps(dict(result=report['result'], report_sha256=digest(args.report))))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
