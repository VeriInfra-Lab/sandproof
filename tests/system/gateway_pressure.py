"""Real gateway admission and idle budgets, with original sync/async SDKs.

Use an isolated service: max_transfers=4, max_transfers_per_vm=3,
control_reserve=1, request/write idle=3 seconds, file idle=5 seconds,
64/96 MiB upload/download budgets. Requires two tenants and fixed base tools.
Raw HTTP is used only to control upload progress precisely. Downloads use
the original SDK stream so a truncated response must surface as its error.
"""

import argparse
import asyncio
import hashlib
from http.client import HTTPResponse
import importlib.metadata
import json
import os
import shlex
import socket
import time
from urllib.parse import urlencode, urlsplit
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import FileNotFoundException, RateLimitException, SandboxException
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_command_input import call
from sdk_server_smoke import options


MIB = 1024 * 1024
DOWNLOAD_BYTES = 64 * MIB
PREFIX = b'X' * (128 * 1024)


async def health(sandbox, url):
    assert (await call(sandbox.get_info)).state == 'running'
    async with httpx.AsyncClient(base_url=url, timeout=10, trust_env=False) as client:
        response = await client.get('/health', headers={'X-Access-Token': sandbox._envd_access_token})
        assert response.status_code == 204, response.status_code


async def healthy(sandbox, path):
    assert (await call(sandbox.commands.run, 'printf pressure-healthy', timeout=15)).stdout == 'pressure-healthy'
    await call(sandbox.files.write, path, b'healthy\n', request_timeout=15)
    assert await call(sandbox.files.read, path) == 'healthy\n'


async def limited(function, *args, **kwargs):
    try:
        await call(function, *args, **kwargs)
    except RateLimitException:
        return
    raise AssertionError('data call did not fail with the SDK rate-limit error')


async def wait_capacity(sandbox, path, timeout=12):
    deadline = time.monotonic() + timeout
    rejections = 0
    while True:
        try:
            info = await call(sandbox.files.get_info, path, request_timeout=5)
            return info, rejections
        except RateLimitException:
            rejections += 1
            assert time.monotonic() < deadline, 'transfer capacity was not released'
            await asyncio.sleep(0.05)


async def start_input(sandbox, marker, tracked):
    ready = asyncio.Event()
    output = ''

    def on_stdout(chunk):
        nonlocal output
        output += chunk
        if 'ready\n' in output:
            ready.set()

    command = (f"printf 'once\\n' >> {shlex.quote(marker)}; "
               "exec python3 -u -c 'import sys; print(\"ready\"); print(sys.stdin.buffer.read().hex())'")
    handle = await call(sandbox.commands.run, command, stdin=True, background=True,
                        timeout=60, on_stdout=on_stdout)
    item = {'handle': handle, 'done': False, 'sandbox': sandbox, 'marker': marker}
    tracked.append(item)
    if isinstance(sandbox, AsyncSandbox):
        await asyncio.wait_for(ready.wait(), timeout=10)
    else:
        def consume_ready():
            for stdout, _stderr, _pty in handle:
                if stdout:
                    on_stdout(stdout)
                if ready.is_set():
                    return
            raise AssertionError('command ended before its stdin barrier')
        await call(consume_ready)
    return item


async def end_input(item, content=b''):
    handle = item['handle']
    if content:
        await call(handle.send_stdin, content, request_timeout=10)
    await call(handle.close_stdin, request_timeout=10)
    result = await call(handle.wait)
    assert (result.stdout, result.stderr, result.exit_code) == ('ready\n' + content.hex() + '\n', '', 0)
    item['done'] = True


async def cleanup_handles(tracked):
    errors = []
    for item in tracked:
        try:
            if not item['done']:
                await call(item['handle'].kill)
            await call(item['handle'].disconnect)
        except Exception as error:
            errors.append(error)
    if errors:
        raise ExceptionGroup('command cleanup failed', errors)


async def admission(first, second, directory, url):
    tracked = []
    refused_file = directory + '/refused-file'
    refused_start = directory + '/refused-start'
    try:
        a = await start_input(first, directory + '/a', tracked)
        b = await start_input(first, directory + '/b', tracked)
        # Both successful Start replies are the admission barriers. These
        # processes cannot finish until EOF/signal, regardless of scheduling.
        await limited(first.files.write, refused_file, b'must not arrive')
        await limited(first.commands.run, f'touch {shlex.quote(refused_start)}')
        await health(first, url)
        await healthy(second, directory + '/other-tenant-healthy')
        c = await start_input(second, directory + '/c', tracked)
        # Second has only one data call, below its per-instance allowance;
        # this rejection therefore exercises the global allowance of three.
        await limited(second.files.write, refused_file, b'must not arrive')
        await health(first, url)
        await health(second, url)
        # SendInput and CloseStdin must work through reserved capacity while
        # ordinary data requests are globally saturated.
        await end_input(a, b'control input survives saturation')
        await wait_capacity(second, directory)
        await healthy(second, directory + '/after-eof')
        d = await start_input(second, directory + '/d', tracked)
        await limited(first.files.read, directory + '/a')
        assert await call(b['handle'].kill), 'reserved SendSignal failed'
        try:
            await call(b['handle'].wait)
        except CommandExitException as error:
            assert error.exit_code != 0
        else:
            raise AssertionError('killed command completed successfully')
        b['done'] = True
        await wait_capacity(first, directory)
        await healthy(first, directory + '/after-signal')
        await end_input(c)
        await end_input(d)
        for sandbox in (first, second):
            await wait_capacity(sandbox, directory)
            assert not await call(sandbox.files.exists, refused_file)
        assert not await call(first.files.exists, refused_start)
        for item in tracked:
            assert await call(item['sandbox'].files.read, item['marker']) == 'once\n'
        return {'per_vm_and_global_429': True, 'reserved_health_input_eof_signal': True,
                'rejected_side_effects': False, 'capacity_after_eof_and_signal': True}
    finally:
        await cleanup_handles(tracked)


def connection(url):
    address = urlsplit(url)
    assert address.scheme == 'http' and address.hostname and address.port
    sock = socket.socket()
    sock.settimeout(15)
    sock.connect((address.hostname, address.port))
    return sock


def request_head(sock, sandbox, method, path, extra=''):
    assert all(c in '0123456789abcdefABCDEF' for c in sandbox._envd_access_token)
    sock.sendall((f'{method} /files?{urlencode({"path": path})} HTTP/1.1\r\n'
                  'Host: localhost\r\nConnection: close\r\n'
                  f'X-Access-Token: {sandbox._envd_access_token}\r\n{extra}\r\n').encode())


def chunk(sock, body):
    sock.sendall(f'{len(body):x}\r\n'.encode() + body + b'\r\n')


async def guest_prefix(sandbox, path):
    deadline = time.monotonic() + 5
    while True:
        try:
            size = (await call(sandbox.files.get_info, path)).size
        except FileNotFoundException:
            size = 0
        if size:
            assert size <= len(PREFIX)
            return size
        assert time.monotonic() < deadline, 'upload never reached the guest'
        await asyncio.sleep(0.02)


def read_response(sock):
    response = HTTPResponse(sock)
    response.begin()
    response.read()
    return response.status


def progressing_upload(sandbox, url, path):
    body = b'X' * (64 * 1024)
    with connection(url) as sock:
        request_head(sock, sandbox, 'POST', path,
                     'Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n')
        started = time.monotonic()
        for index in range(6):
            if index:
                # Deliberate cadence: total duration exceeds the configured
                # idle budget, each interval stays below it. Not a race barrier.
                time.sleep(1)
            chunk(sock, body)
        sock.sendall(b'0\r\n\r\n')
        assert read_response(sock) == 200
        return {'bytes': len(body) * 6, 'elapsed_seconds': time.monotonic() - started,
                'sha256': hashlib.sha256(body * 6).hexdigest()}


async def finish_truncated(chunks, is_async, received):
    try:
        while True:
            data = await anext(chunks, None) if is_async else await call(next, chunks, None)
            if data is None:
                break
            assert data == b'X' * len(data)
            received += len(data)
            assert received < DOWNLOAD_BYTES
    except (httpx.RemoteProtocolError, httpx.ReadError) as error:
        error_name = type(error).__name__
    else:
        raise AssertionError('stalled download ended as complete success')
    assert 0 < received < DOWNLOAD_BYTES
    return received, error_name


async def stalled_download(sandbox, control, directory, url):
    path = directory + '/large.bin'
    await call(sandbox.commands.run,
               f"head -c {DOWNLOAD_BYTES} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30)
    assert (await call(sandbox.files.get_info, path)).size == DOWNLOAD_BYTES
    tracked = []
    try:
        quiet = await start_input(sandbox, directory + '/quiet', tracked)
        stream = await call(sandbox.files.read, path, format='stream', request_timeout=30)
        is_async = hasattr(stream, '__aenter__')
        chunks = await stream.__aenter__() if is_async else await call(stream.__enter__)
        try:
            first = await anext(chunks) if is_async else await call(next, chunks)
            assert first and first == b'X' * len(first)
            await limited(sandbox.files.get_info, path)
            await health(sandbox, url)
            await healthy(control, directory + '/slow-download-control')
            started = time.monotonic()
            # Stop consuming SDK chunks. The initial 429 confirms the stream
            # is still admitted; observe its release before reading further.
            info, rejections = await wait_capacity(sandbox, path)
            assert info.size == DOWNLOAD_BYTES
            elapsed = time.monotonic() - started
            received, error_name = await finish_truncated(chunks, is_async, len(first))
        finally:
            if is_async:
                await stream.__aexit__(None, None, None)
            else:
                await call(stream.__exit__, None, None, None)
        # The quiet command survives the stalled file transfer and finishes normally.
        await end_input(quiet, b'quiet command stayed alive')
        return {'received_bytes': received, 'intended_bytes': DOWNLOAD_BYTES,
                'capacity_release_seconds': elapsed, '429_while_stalled': rejections,
                'sdk_error': error_name}
    finally:
        await cleanup_handles(tracked)


async def stalled_headers(sandbox, control, directory):
    fifo = directory + '/header-gate'
    await call(sandbox.commands.run, f'mkfifo {shlex.quote(fifo)}', timeout=15)
    tracked = []
    try:
        quiet = await start_input(sandbox, directory + '/header-quiet', tracked)
        started = time.monotonic()
        # The fixed envd opens this FIFO before sending headers. No writer
        # exists until after the gateway's header deadline has been observed.
        pending = asyncio.create_task(call(sandbox.files.read, fifo, request_timeout=15))
        try:
            await healthy(control, directory + '/header-idle-control')
            try:
                await pending
            except SandboxException as error:
                assert str(error).startswith('504:'), type(error).__name__
            else:
                raise AssertionError('blocked guest file unexpectedly completed')
            elapsed = time.monotonic() - started
        finally:
            # Wake the guest's blocked open, then remove only our FIFO. The
            # bounded writer also makes an unexpected missing reader fail.
            writer = f"printf '%s' {'X' * 1024} > {shlex.quote(fifo)}"
            await call(sandbox.commands.run,
                       f'timeout 5 sh -c {shlex.quote(writer)}; status=$?; rm -f -- {shlex.quote(fifo)}; exit "$status"',
                       timeout=10)
            if not pending.done():
                await pending
        await end_input(quiet, b'quiet command outlived file header idle')
        return {'status': 504, 'elapsed_seconds': elapsed, 'quiet_command_completed': True}
    finally:
        await cleanup_handles(tracked)


async def transport_cases(sandbox, control, directory, url):
    path = directory + '/idle-upload'
    with connection(url) as sock:
        request_head(sock, sandbox, 'POST', path,
                     'Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n')
        chunk(sock, PREFIX)
        prefix = await guest_prefix(sandbox, path)
        await healthy(control, directory + '/idle-upload-control')
        assert await call(read_response, sock) == 408
    await healthy(sandbox, directory + '/after-idle-upload')
    progress_path = directory + '/progress-upload'
    progressing = await call(progressing_upload, sandbox, url, progress_path)
    content = await call(sandbox.files.read, progress_path, format='bytes')
    assert len(content) == progressing['bytes'] and hashlib.sha256(content).hexdigest() == progressing['sha256']
    stalled = await stalled_download(sandbox, control, directory, url)
    headers = await stalled_headers(sandbox, control, directory)
    await healthy(sandbox, directory + '/after-stalled-download')
    await healthy(control, directory + '/after-transport-control')
    return {'idle_upload_status': 408, 'observed_guest_prefix': prefix,
            'progressing_upload': progressing, 'stalled_download': stalled,
            'stalled_headers': headers}


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandboxes = []
        try:
            for key in ('SANDPROOF_TEST_API_KEY', 'SANDPROOF_TEST_FOREIGN_API_KEY'):
                owner = options(os.environ[key], args.api_url, args.sandbox_url)
                sandboxes.append(await call(cls.create, template='base', timeout=300,
                                            allow_internet_access=False, **owner))
            first, second = sandboxes
            directory = '/tmp/pressure-' + uuid.uuid4().hex
            for sandbox in sandboxes:
                await call(sandbox.files.make_dir, directory)
            result = await admission(first, second, directory, args.sandbox_url)
            result.update(await transport_cases(first, second, directory, args.sandbox_url))
            result['client'] = cls.__name__
            results.append(result)
        finally:
            errors = []
            for sandbox in sandboxes:
                try:
                    assert await call(sandbox.kill)
                except Exception as error:
                    errors.append(error)
            if errors:
                raise ExceptionGroup('sandbox cleanup failed', errors)
    print(json.dumps({'sdk_version': '2.51.0', 'cases': results,
                      'scope': 'real two-tenant admission, reserved control and transport idle; not final deployment budgets or exhaustive races'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
