"""Two finite final-profile gateway cases against operator-owned live VMs.

This client does not launch/kill VMs or services, create/join/remove cgroups, or
change deployment budgets. The outer approved host supervisor must provide two
isolated instances with at least 600 seconds remaining, observe their real host
identities, and reclaim them and the service/relays afterwards. This probe checks
only synchronous original-SDK network behavior and the recorded service leaf;
it does not test the 13000-second expiry, async SDK, or final host reclamation.
Credentials come only from SANDPROOF_TEST_API_KEY / _FOREIGN_API_KEY.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
from contextlib import nullcontext
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import shlex
import socket
import stat
import subprocess
import sys
import time
from urllib.parse import urlsplit
import uuid

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import check_kvm_host as host
from tools import service_cgroup

MIB = 1024 * 1024
DOWNLOAD_BYTES = 64 * MIB
STREAM_COUNT = 6
PAUSE_SECONDS = 60
CHUNK_SECONDS = 8
CHUNK_COUNT = 6
HTTP_TIMEOUT = 90
IDLE_MAXIMUM_SECONDS = 45
FINAL_TRANSFER = dict(file_upload_bytes=64 * MIB, file_download_bytes=96 * MIB,
                      command_input_bytes=MIB, command_output_bytes=32 * MIB,
                      rpc_request_bytes=65536, rpc_response_bytes=MIB,
                      max_transfers=16, max_transfers_per_vm=8, control_reserve=1,
                      request_idle_seconds=13000, file_idle_seconds=30,
                      write_idle_seconds=13000)


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def private_json(path):
    path = host.trusted_path(Path(path))
    meta = path.stat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_mode & 0o077 or meta.st_size > 256 * 1024:
        raise ValueError('private root-owned JSON required')
    return json.loads(path.read_bytes(), object_pairs_hook=host.unique_keys)


def loopback_url(value):
    address = urlsplit(value)
    if (address.scheme != 'http' or address.hostname != '127.0.0.1'
            or not address.port or address.username or address.password
            or address.path not in ('', '/') or address.query or address.fragment):
        raise ValueError('explicit HTTP IPv4 loopback origin required')
    return value.rstrip('/')


def validate_configuration(settings):
    actual = settings.get('data_transfer')
    reference = json.loads((ROOT / 'benchmarks/local-service-budgets.json').read_bytes())['data_transfer']
    if (reference != FINAL_TRANSFER or actual != FINAL_TRANSFER
            or any(type(value) is not int for value in actual.values())):
        raise ValueError('unchanged complete final data_transfer configuration required')
    if settings.get('max_active_global', 0) < 2:
        raise ValueError('two externally supervised live instances required')
    return dict(actual)


def start_ticks(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    except FileNotFoundError:
        return None
    return None if fields[0] == 'Z' else int(fields[19])


def process_binding(pid, ticks, config):
    if start_ticks(pid) != ticks:
        raise ValueError('server PID/start identity changed')
    arguments = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
    if arguments[1:] != [os.fsencode(config), b'']:
        raise ValueError('server argv does not bind the exact supplied configuration')
    executable = Path(f'/proc/{pid}/exe')
    meta = executable.stat()
    return dict(pid=pid, start_ticks=ticks, executable_device=meta.st_dev,
                executable_inode=meta.st_ino, executable_sha256=digest(executable))


def assert_memory(baseline, snapshot):
    if (snapshot['identity'] != baseline['identity'] or snapshot['limits'] != baseline['limits']
            or snapshot['memory_peak'] > int(baseline['limits']['memory.max'])
            or snapshot['memory_current'] > int(baseline['limits']['memory.max'])):
        raise AssertionError('recorded service identity or hard memory budget violated')
    for event in ('oom', 'oom_kill', 'oom_group_kill'):
        if snapshot['memory_events'].get(event, 0) != baseline['memory_events'].get(event, 0):
            raise AssertionError('service cgroup OOM counter increased')


class ServiceObserver:
    def __init__(self, record, pid, ticks):
        self.record, self.pid, self.ticks = record, pid, ticks
        self.baseline = service_cgroup.observe(record, pids=[pid])
        self.samples = []

    def sample(self, label):
        if start_ticks(self.pid) != self.ticks:
            raise ValueError('original service process exited or was replaced')
        snapshot = service_cgroup.observe(self.record, pids=[self.pid])
        identities = []
        for pid in snapshot['pids']:
            ticks = start_ticks(pid)
            if ticks is not None:
                identities.append(dict(pid=pid, start_ticks=ticks))
        result = dict(label=label, monotonic_seconds=time.monotonic(),
                      process_identities=identities, **snapshot)
        self.samples.append(result)
        assert_memory(self.baseline, snapshot)
        return result


def connection(url):
    address = urlsplit(loopback_url(url))
    return socket.create_connection((address.hostname, address.port), timeout=HTTP_TIMEOUT)


def progressing_upload(sandbox, url, path, helpers, *, clock=time.monotonic, sleep=time.sleep,
                       shared_socket=None):
    body = b'X' * 65536
    with connection(url) if shared_socket is None else nullcontext(shared_socket) as sock:
        helpers.request_head(sock, sandbox, 'POST', path,
                             'Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n')
        started = clock()
        for index in range(CHUNK_COUNT):
            if index:
                sleep(CHUNK_SECONDS)  # Fixed workload cadence, not a race barrier.
            helpers.chunk(sock, body)
        sock.sendall(b'0\r\n\r\n')
        if helpers.read_response(sock) != 200:
            raise AssertionError('valid continuing upload failed')
    elapsed = clock() - started
    if elapsed < (CHUNK_COUNT - 1) * CHUNK_SECONDS:
        raise AssertionError('progress workload did not cross the real file idle window')
    return dict(bytes=len(body) * CHUNK_COUNT, elapsed_seconds=elapsed,
                intervals_seconds=[CHUNK_SECONDS] * (CHUNK_COUNT - 1),
                sha256=hashlib.sha256(body * CHUNK_COUNT).hexdigest())


async def quiet_input(sandbox, marker, tracked, helpers):
    command = (f"printf 'once\\n' >> {shlex.quote(marker)}; "
               "exec python3 -u -c 'import sys; print(\"ready\"); print(sys.stdin.buffer.read().hex())'")
    handle = await helpers.call(sandbox.commands.run, command, stdin=True, background=True, timeout=180)
    item = dict(handle=handle, done=False, sandbox=sandbox, marker=marker)
    tracked.append(item)

    def ready():
        output = ''
        for stdout, _stderr, _pty in handle:
            output += stdout or ''
            if 'ready\n' in output:
                return
        raise AssertionError('quiet command ended before its admission barrier')

    await asyncio.wait_for(helpers.call(ready), timeout=15)
    return item


async def finish_quiet(item, content, helpers):
    await helpers.call(item['handle'].close_stdin, request_timeout=10)
    result = await helpers.call(item['handle'].wait)
    if (result.stdout, result.stderr, result.exit_code) != ('ready\n' + content.hex() + '\n', '', 0):
        raise AssertionError('quiet command did not survive unchanged')
    item['done'] = True


async def idle_and_progress(target, control, directory, url, helpers):
    from e2b.exceptions import SandboxException
    upload, fifo, progress = (directory + '/' + name for name in ('idle-upload', 'fifo', 'progress'))
    result = {}
    with connection(url) as sock:
        helpers.request_head(sock, target, 'POST', upload,
                             'Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n')
        helpers.chunk(sock, helpers.PREFIX)
        started = time.monotonic()
        prefix = await helpers.guest_prefix(target, upload)
        await helpers.healthy(control, directory + '/control')
        status = await helpers.call(helpers.read_response, sock)
        elapsed = time.monotonic() - started
        if status != 504 or not 29 <= elapsed <= IDLE_MAXIMUM_SECONDS:
            raise AssertionError('real prefixed upload did not expire at the final file-header idle boundary')
        result['upload_idle'] = dict(status=status, guest_prefix_bytes=prefix, elapsed_seconds=elapsed)

    await helpers.call(target.commands.run, 'mkfifo -- ' + shlex.quote(fifo), timeout=10)
    started = time.monotonic()
    read = asyncio.create_task(helpers.call(target.files.read, fifo, request_timeout=HTTP_TIMEOUT))
    try:
        await helpers.healthy(control, directory + '/control')
        try:
            await read
        except SandboxException as error:
            if not str(error).startswith('504:'):
                raise AssertionError('FIFO did not return the real gateway 504') from None
        else:
            raise AssertionError('FIFO without a writer completed successfully')
        elapsed = time.monotonic() - started
        if not 29 <= elapsed <= IDLE_MAXIMUM_SECONDS:
            raise AssertionError('FIFO did not cross the unchanged 30-second idle boundary')
        result['fifo_idle'] = dict(status=504, elapsed_seconds=elapsed, writer_before_response=False)
    finally:
        # envd can still be blocked in FIFO open after the gateway cancels. Wake
        # only this exact FIFO, bounded by guest timeout, before removing it.
        wake = f'timeout 5 sh -c {shlex.quote("printf wake > " + shlex.quote(fifo))}; rm -f -- {shlex.quote(fifo)}'
        await helpers.call(target.commands.run, wake, timeout=10)
        if not read.done():
            read.cancel()
        await asyncio.gather(read, return_exceptions=True)

    with connection(url) as sock:
        pending = asyncio.create_task(helpers.call(progressing_upload, target, url, progress, helpers,
                                                   shared_socket=sock))
        try:
            prefix = await helpers.guest_prefix(target, progress)
            await helpers.healthy(control, directory + '/control')
            result['progress_upload'] = await pending
            result['progress_upload'].update(guest_prefix_bytes=prefix, control_healthy_during_progress=True)
        finally:
            if not pending.done():
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                sock.close()
                pending.cancel()
            await asyncio.gather(pending, return_exceptions=True)
    payload = await helpers.call(target.files.read, progress, format='bytes', request_timeout=15)
    if len(payload) != result['progress_upload']['bytes'] or hashlib.sha256(payload).hexdigest() != result['progress_upload']['sha256']:
        raise AssertionError('progress upload guest contents differ')
    await helpers.healthy(control, directory + '/control')
    return result


def consume(chunks, initial):
    count = len(initial)
    sha = hashlib.sha256(initial)
    for data in chunks:
        if not data or data != b'X' * len(data):
            raise AssertionError('download contents differ')
        count += len(data)
        if count > DOWNLOAD_BYTES:
            raise AssertionError('download exceeded fixture size')
        sha.update(data)
    if count != DOWNLOAD_BYTES:
        raise AssertionError('paused download truncated')
    return dict(bytes=count, sha256=sha.hexdigest())


async def paused_downloads(target, control, directory, url, helpers, observer, *, cleanup_errors=None):
    path, marker = directory + '/large.bin', directory + '/quiet'
    await helpers.call(target.commands.run,
                       f"head -c {DOWNLOAD_BYTES} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30)
    if (await helpers.call(target.files.get_info, path)).size != DOWNLOAD_BYTES:
        raise AssertionError('64-MiB guest fixture missing')
    tracked, streams, tasks = [], [], []
    try:
        quiet = await quiet_input(target, marker, tracked, helpers)
        for _ in range(STREAM_COUNT):
            stream = await helpers.call(target.files.read, path, format='stream', request_timeout=180)
            item = dict(stream=stream, entered=False)
            streams.append(item)
            item['chunks'] = await helpers.call(stream.__enter__)
            item['entered'] = True
            item['first'] = await helpers.call(next, item['chunks'])
            if not item['first'] or item['first'] != b'X' * len(item['first']):
                raise AssertionError('download admission prefix missing')
        # Six real response prefixes plus quiet command's ready event, not a
        # sleep, establish all seven ordinary per-instance permits are held.
        await helpers.limited(target.files.get_info, path)
        await helpers.health(target, url)
        content = b'reserved input during six paused downloads'
        await helpers.call(quiet['handle'].send_stdin, content, request_timeout=10)
        await helpers.healthy(control, directory + '/control')
        started = time.monotonic()
        observer.sample('seven-admitted-before-pause')
        while time.monotonic() - started < PAUSE_SECONDS:
            await asyncio.sleep(min(1, PAUSE_SECONDS - (time.monotonic() - started)))
            observer.sample('paused-downloads')
        elapsed = time.monotonic() - started
        await helpers.limited(target.files.get_info, path)
        await helpers.health(target, url)
        await helpers.healthy(control, directory + '/control')
        await finish_quiet(quiet, content, helpers)
        # Resume together: sequentially draining six bodies would introduce an
        # unrelated producer-idle delay for those still waiting their turn.
        tasks = [asyncio.create_task(helpers.call(consume, item['chunks'], item['first'])) for item in streams]
        results = await asyncio.wait_for(asyncio.gather(*tasks), timeout=90)
        observer.sample('downloads-resumed-complete')
        await helpers.wait_capacity(target, path)
        return dict(stream_count=STREAM_COUNT, fixture_bytes=DOWNLOAD_BYTES,
                    paused_seconds=elapsed, initial_bytes=[len(item['first']) for item in streams],
                    per_vm_429_before_and_after_pause=True, reserved_health_and_input=True,
                    control_instance_healthy=True, quiet_command_completed=True, downloads=results)
    finally:
        for task in tasks:
            if not task.done():
                task.cancel()
        # Close sockets before joining cancelled reader tasks. SDK HTTP timeouts
        # also bound any sync worker that cannot immediately be interrupted.
        errors = []
        for item in streams:
            try:
                if item['entered']:
                    await helpers.call(item['stream'].__exit__, None, None, None)
            except Exception as error:
                errors.append(error)
        await asyncio.gather(*tasks, return_exceptions=True)
        try:
            await helpers.cleanup_handles(tracked)
        except Exception as error:
            errors.append(error)
        if errors:
            if cleanup_errors is not None:
                cleanup_errors.extend(type(error).__name__ for error in errors)
            raise ExceptionGroup('exact client transfer cleanup failed', errors)


async def exercise(args, report):
    # Import the pinned original SDK only for real execution, not unit discovery.
    import gateway_pressure as helpers
    from e2b import Sandbox
    from sdk_server_smoke import options

    api, url = loopback_url(args.api_url), loopback_url(args.sandbox_url)
    config = host.trusted_path(args.config)
    settings = private_json(config)
    report['data_transfer'] = validate_configuration(settings)
    if api != url or settings.get('bind') != url.removeprefix('http://'):
        raise ValueError('both client origins must bind the supplied server listener')
    report['config_sha256'] = digest(config)
    report['server'] = process_binding(args.server_pid, args.server_start_ticks, config)
    observer = ServiceObserver(args.service_cgroup_record, args.server_pid, args.server_start_ticks)
    report['service_baseline'] = observer.baseline
    report['service_samples'] = observer.samples
    report['sdk_version'] = importlib.metadata.version('e2b')
    if report['sdk_version'] != '2.51.0':
        raise ValueError('locked original SDK 2.51.0 required')
    keys = [os.environ.get(name) for name in ('SANDPROOF_TEST_API_KEY', 'SANDPROOF_TEST_FOREIGN_API_KEY')]
    if not all(keys) or keys[0] == keys[1]:
        raise ValueError('two explicit distinct isolated-tenant credentials required')
    instances = []
    for identity, key in zip((args.target_sandbox_id, args.control_sandbox_id), keys):
        if not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', identity):
            raise ValueError('explicit sandbox identity required')
        instance = await helpers.call(Sandbox.connect, identity,
                                      **(options(key, api, url) | {'request_timeout': HTTP_TIMEOUT}))
        if instance.sandbox_id != identity:
            raise ValueError('SDK returned a different sandbox identity')
        info = await helpers.call(instance.get_info)
        remaining = (info.end_at - datetime.now(timezone.utc)).total_seconds()
        if info.state != 'running' or remaining < 300:
            raise ValueError('outer-supervised running VM needs at least 300 seconds remaining')
        report.setdefault('initial_remaining_seconds', []).append(remaining)
        instances.append(instance)
    if instances[0].sandbox_id == instances[1].sandbox_id:
        raise ValueError('two distinct externally owned instances required')
    target, control = instances
    directory = '/tmp/gateway-slow-' + uuid.uuid4().hex
    created = []
    try:
        for instance in instances:
            await helpers.call(instance.commands.run, 'mkdir -- ' + shlex.quote(directory), timeout=10)
            created.append(instance)
        report['sandbox_ids'] = [instance.sandbox_id for instance in instances]
        report['guest_directory'] = directory
        observer.sample('before-cases')
        report['cases']['file_idle_and_progress'] = await idle_and_progress(target, control, directory, url, helpers)
        report['cases']['paused_download_memory'] = await paused_downloads(
            target, control, directory, url, helpers, observer, cleanup_errors=report['transfer_cleanup_errors'])
        if digest(config) != report['config_sha256'] or process_binding(args.server_pid, args.server_start_ticks, config) != report['server']:
            raise ValueError('original server/config identity changed during probe')
        observer.sample('after-cases')
    finally:
        for instance in created:
            try:
                names = ('idle-upload', 'fifo', 'progress', 'large.bin', 'quiet', 'control')
                exact = ' '.join(shlex.quote(directory + '/' + name) for name in names)
                result = await helpers.call(instance.commands.run,
                                            f'rm -f -- {exact}; rmdir -- {shlex.quote(directory)}', timeout=15)
                if result.exit_code != 0:
                    raise AssertionError('exact guest fixture cleanup failed')
            except Exception as error:
                report['client_cleanup_errors'].append(type(error).__name__)
        if report['client_cleanup_errors']:
            raise AssertionError('guest fixture cleanup incomplete; outer supervisor must reclaim exact VMs')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('api-url', 'sandbox-url', 'target-sandbox-id', 'control-sandbox-id'):
        parser.add_argument('--' + field, required=True)
    for field in ('config', 'service-cgroup-record', 'report'):
        parser.add_argument('--' + field, type=Path, required=True)
    parser.add_argument('--server-pid', type=int, required=True)
    parser.add_argument('--server-start-ticks', type=int, required=True)
    args = parser.parse_args(argv)
    if os.geteuid() != 0 or sys.platform != 'linux':
        parser.error('root Linux with an outer approved host supervisor required')
    service_cgroup.private(args.report.parent)
    if args.report.is_symlink() or not args.report.is_absolute():
        parser.error('new absolute private report path required')
    fd = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    report = dict(schema_version=1, scope='two_finite_final_profile_sync_gateway_cases',
                  result='incomplete', cases={}, client_cleanup_errors=[], transfer_cleanup_errors=[],
                  host_cleanup='not_checked_here_outer_supervisor_required',
                  not_tested=['async_sdk', '13000_second_idle_expiry', 'final_host_reclamation'])
    code = 1
    try:
        if subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT).strip():
            raise ValueError('reviewed clean source checkout required')
        report['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        report['source_tree'] = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip()
        report['probe_sha256'] = digest(__file__)
        asyncio.run(exercise(args, report))
        report['result'], code = 'passed', 0
    except Exception as error:
        report['failure_type'] = type(error).__name__  # Never serialize SDK errors/tokens.
    finally:
        with os.fdopen(fd, 'w') as output:
            json.dump(report, output, sort_keys=True)
            output.write('\n')
            output.flush()
            os.fsync(output.fileno())
    print(json.dumps(dict(result=report['result'], report=str(args.report), report_sha256=digest(args.report))))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
