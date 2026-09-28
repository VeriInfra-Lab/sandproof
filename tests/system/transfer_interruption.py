"""Real sync/async SDK uploads and downloads interrupted by kill or expiry.

Requires an isolated streaming service with 64/96 MiB file budgets, request /
file / write idle timers of at least 30 seconds, and two VMs per tenant. Uses
original SDK calls plus a bounded producer barrier; no host fault injection.
"""

import argparse
import asyncio
import importlib.metadata
import inspect
from io import RawIOBase
import json
import os
import shlex
import threading
import time
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import FileNotFoundException, SandboxException

from gateway_async_stream_limits import async_stream_failure
from gateway_stream_limits import stream_failure
from sdk_server_smoke import options


MIB = 1024 * 1024
UPLOAD_BYTES = 2 * MIB
UPLOAD_PREFIX = 256 * 1024
DOWNLOAD_BYTES = 64 * MIB


async def call(function, *args, **kwargs):
    if inspect.iscoroutinefunction(function):
        return await function(*args, **kwargs)
    result = await asyncio.to_thread(function, *args, **kwargs)
    return await result if inspect.isawaitable(result) else result


class PausedUpload(RawIOBase):
    def __init__(self):
        self.sent = 0
        self.blocked = threading.Event()
        self.release = threading.Event()

    def read(self, size=-1):
        if self.sent == UPLOAD_PREFIX and not self.release.is_set():
            self.blocked.set()
            if not self.release.wait(timeout=45):
                raise TimeoutError('test producer barrier was not released')
        boundary = UPLOAD_PREFIX if self.sent < UPLOAD_PREFIX else UPLOAD_BYTES
        count = min(64 * 1024, boundary - self.sent, size if size >= 0 else 64 * 1024)
        self.sent += count
        return b'X' * count


async def actual_prefix(sandbox, path, transfer, producer):
    assert await asyncio.to_thread(producer.blocked.wait, 5), 'upload did not reach producer barrier'
    deadline = time.monotonic() + 5
    while True:
        assert not transfer.done(), 'upload finished before lifecycle interruption'
        try:
            size = (await call(sandbox.files.get_info, path)).size
        except FileNotFoundException:
            size = 0
        if size > 0:
            assert size <= UPLOAD_PREFIX
            return size
        assert time.monotonic() < deadline, 'no actual guest write observed before interruption'
        await asyncio.sleep(0.02)


async def healthy(control, marker):
    assert (await call(control.get_info)).state == 'running'
    assert (await call(control.commands.run, 'printf control-healthy', timeout=15)).stdout == 'control-healthy'
    await call(control.files.write, marker, b'control unchanged\n', request_timeout=15)
    assert await call(control.files.read, marker) == 'control unchanged\n'


async def revoked(sandbox, url):
    async with httpx.AsyncClient(base_url=url, trust_env=False, timeout=10) as client:
        headers = {'X-Access-Token': sandbox._envd_access_token}
        for method, path in (('GET', '/health'), ('GET', '/files?path=/tmp/old'),
                             ('POST', '/files?path=/tmp/old'), ('POST', '/process.Process/Start')):
            response = await client.request(method, path, headers=headers, content=b'')
            assert response.status_code == 401, (method, path, response.status_code)


async def interrupted_monitor(monitor, timeout=10):
    done, _pending = await asyncio.wait({monitor}, timeout=timeout)
    assert done, 'running command connection did not observe VM termination'
    try:
        monitor.result()
    except Exception as error:
        if not async_stream_failure(error):
            raise
        return type(error).__name__
    raise AssertionError('monitor command ended successfully despite VM destruction')


async def trigger(sandbox, mode, monitor):
    started = time.monotonic()
    if mode == 'kill':
        assert await call(sandbox.kill), 'kill was not confirmed'
    else:
        await call(sandbox.set_timeout, 2)
    error = await interrupted_monitor(monitor)
    return {'trigger': mode, 'monitor_error': error, 'elapsed_seconds': time.monotonic() - started}


def upload_failure(error):
    return (type(error) in (httpx.ReadError, httpx.WriteError, httpx.RemoteProtocolError)
            or (type(error) is SandboxException and str(error).startswith('502:')))


async def upload_case(sandbox, mode, monitor, directory):
    path = directory + '/upload.bin'
    producer = PausedUpload()
    transfer = asyncio.create_task(call(sandbox.files.write, path, producer, request_timeout=60))
    try:
        prefix = await actual_prefix(sandbox, path, transfer, producer)
        result = await trigger(sandbox, mode, monitor)
        producer.release.set()
        done, _pending = await asyncio.wait({transfer}, timeout=15)
        assert done, 'upload did not finish after producer release'
        try:
            transfer.result()
        except Exception as error:
            if not upload_failure(error):
                raise
            result['upload_error'] = type(error).__name__
        else:
            raise AssertionError('interrupted upload returned complete success')
        result['observed_guest_prefix_bytes'] = prefix
        result['intended_file_bytes'] = UPLOAD_BYTES
        return result
    finally:
        producer.release.set()
        if not transfer.done():
            done, _pending = await asyncio.wait({transfer}, timeout=15)
            assert done, 'upload task still active after test cleanup'
        # Retrieve any exception without allowing an unobserved task warning.
        if not transfer.cancelled():
            transfer.exception()


async def download_case(sandbox, mode, monitor, directory):
    stream = await call(sandbox.files.read, directory + '/download.bin', format='stream', request_timeout=60)
    is_async = hasattr(stream, '__aenter__')
    chunks = await stream.__aenter__() if is_async else await call(stream.__enter__)
    received = 0
    try:
        chunk = await anext(chunks) if is_async else await call(next, chunks)
        assert chunk and chunk == b'X' * len(chunk)
        received += len(chunk)
        assert received < DOWNLOAD_BYTES, 'download completed before interruption barrier'
        result = await trigger(sandbox, mode, monitor)
        try:
            while True:
                chunk = await anext(chunks, None) if is_async else await call(next, chunks, None)
                if chunk is None:
                    break
                assert chunk == b'X' * len(chunk)
                received += len(chunk)
        except Exception as error:
            if not stream_failure(error):
                raise
            result['download_error'] = type(error).__name__
        else:
            raise AssertionError('interrupted download returned complete success')
        assert 0 < received < DOWNLOAD_BYTES
        result['received_file_bytes'] = received
        result['intended_file_bytes'] = DOWNLOAD_BYTES
        return result
    finally:
        if is_async:
            await stream.__aexit__(None, None, None)
        else:
            await call(stream.__exit__, None, None, None)


async def client_cases(cls, owner, args):
    control = await call(cls.create, template='base', timeout=300, allow_internet_access=False, **owner)
    marker = f'/tmp/transfer-control-{uuid.uuid4().hex}'
    results = []
    try:
        for direction in ('upload', 'download'):
            for mode in ('kill', 'expiry'):
                sandbox = await call(cls.create, template='base', timeout=180, allow_internet_access=False, **owner)
                handle = None
                monitor = None
                completed = False
                try:
                    directory = f'/tmp/transfer-interruption-{uuid.uuid4().hex}'
                    await call(sandbox.files.make_dir, directory)
                    if direction == 'download':
                        await call(sandbox.commands.run,
                                   f"head -c {DOWNLOAD_BYTES} /dev/zero | tr '\\000' X > {shlex.quote(directory + '/download.bin')}", timeout=30)
                        assert (await call(sandbox.files.get_info, directory + '/download.bin')).size == DOWNLOAD_BYTES
                    await healthy(control, marker)
                    handle = await call(sandbox.commands.run, 'sleep 120', background=True, timeout=20)
                    monitor = asyncio.create_task(call(handle.wait))
                    if direction == 'upload':
                        result = await upload_case(sandbox, mode, monitor, directory)
                    else:
                        result = await download_case(sandbox, mode, monitor, directory)
                    await revoked(sandbox, args.sandbox_url)
                    await healthy(control, marker)
                    result.update(client=cls.__name__, direction=direction,
                                  sandbox_id=sandbox.sandbox_id, control_id=control.sandbox_id)
                    results.append(result)
                    completed = True
                finally:
                    # Expiry success is established before cleanup; never use a
                    # manual kill to make the expiry assertion pass.
                    if not completed:
                        await call(sandbox.kill)
                    if handle is not None:
                        await call(handle.disconnect)
                    if monitor is not None:
                        done, _pending = await asyncio.wait({monitor}, timeout=22)
                        assert done, 'monitor remained active after cleanup'
                        if not monitor.cancelled():
                            monitor.exception()
    finally:
        assert await call(control.kill), 'control VM destruction not confirmed'
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        results.extend(await client_cases(cls, owner, args))
    print(json.dumps({'sdk_version': '2.51.0', 'cases': results,
        'scope': 'mid-transfer kill/expiry after observed guest upload or received download bytes; concurrent VM and revoked-token checks; exact host cleanup requires independent audit'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
