"""Original async SDK large-transfer boundaries against an isolated real VM.

Requires explicit data_transfer budgets matching the arguments and the fixed
SDK/base used by gateway_stream_limits.py. Creates and kills its own sandbox.
"""

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
import shlex
import tempfile
import uuid

from e2b import AsyncSandbox
from e2b.exceptions import SandboxException, TimeoutException, format_sandbox_timeout_exception

from gateway_stream_limits import UnknownLengthFile, pattern_digest, stream_failure
from sdk_server_smoke import options


ASYNC_CHUNK_EOF = str(format_sandbox_timeout_exception(
    'Error reading content: request or response body error: '
    'error reading a body from connection: unexpected EOF during chunk size line'))
ASYNC_BODY_EOF = str(format_sandbox_timeout_exception(
    'Error reading content: request or response body error: '
    'error reading a body from connection: end of file before message length reached'))


def async_stream_failure(error):
    if stream_failure(error):
        return True
    # SDK 2.51.0 stores a newly mapped exception in its background event task,
    # then raises it in wait(), losing the ConnectError/transport chain. Accept
    # only these exact observed EOF renderings (at a chunk boundary or within
    # its body); not arbitrary timeouts or messages with a deadline cause.
    return (type(error) is TimeoutException and error.__cause__ is None
            and error.__context__ is None and str(error) in (ASYNC_CHUNK_EOF, ASYNC_BODY_EOF))


async def healthy(sandbox):
    assert (await sandbox.get_info()).state == 'running'
    assert (await sandbox.commands.run('printf async-stream-healthy', timeout=15)).stdout == 'async-stream-healthy'


async def upload_boundary(sandbox, directory, limit):
    cases = []
    for seekable in (True, False):
        for size in (0, limit - 1, limit, limit + 1):
            path = f'{directory}/upload-{seekable}-{size}.bin'
            with tempfile.TemporaryFile() as known:
                if seekable:
                    producer = UnknownLengthFile(size)
                    while chunk := producer.read():
                        known.write(chunk)
                    known.seek(0)
                body = known if seekable else UnknownLengthFile(size)
                error_name = None
                try:
                    await sandbox.files.write(path, body, request_timeout=60)
                except SandboxException as error:
                    if size <= limit or not str(error).startswith('413:'):
                        raise
                    error_name = type(error).__name__
            digest = None
            if size <= limit:
                assert error_name is None
                assert (await sandbox.files.get_info(path)).size == size
                digest = (await sandbox.commands.run(f'sha256sum {shlex.quote(path)}', timeout=30)).stdout.split()[0]
                assert digest == pattern_digest(size)
            else:
                assert error_name is not None, 'oversized upload returned success'
                # The pinned async SDK sends IO as chunks, even if seekable.
                # Either source may leave a partial file; 413 is not rollback.
            await sandbox.commands.run(f'rm -f -- {shlex.quote(path)}', timeout=15)
            await healthy(sandbox)
            cases.append({'file_bytes': size, 'source_seekable': seekable,
                          'sha256': digest, 'upload_error': error_name})
    return cases


async def download_boundary(sandbox, directory, limit):
    path = f'{directory}/download.bin'
    cases = []
    for size in (0, limit - 1, limit, limit + 1):
        await sandbox.commands.run(
            f"head -c {size} /dev/zero | tr '\\000' X > {shlex.quote(path)}", timeout=30)
        assert (await sandbox.files.get_info(path)).size == size
        received = 0
        digest = hashlib.sha256()
        error_name = None
        try:
            async with await sandbox.files.read(path, format='stream', request_timeout=30) as chunks:
                async for chunk in chunks:
                    assert chunk == b'X' * len(chunk)
                    received += len(chunk)
                    digest.update(chunk)
        except Exception as error:
            if size <= limit or not stream_failure(error):
                raise
            error_name = type(error).__name__
        if size <= limit:
            assert received == size
            assert digest.hexdigest() == pattern_digest(size)
        else:
            assert error_name is not None, 'oversized download ended successfully'
            assert 0 < received <= limit
        await healthy(sandbox)
        cases.append({'file_bytes': size, 'received_bytes': received,
                      'sha256': digest.hexdigest() if error_name is None else None,
                      'stream_error': error_name})
    await sandbox.commands.run(f'rm -f -- {shlex.quote(path)}', timeout=15)
    return cases


async def command_overflow(sandbox, directory, limit):
    marker = f'{directory}/overflow-starts'
    received = 0

    # Positive control: a smaller stream must end with a complete result.
    below = limit // 2
    result = await sandbox.commands.run(
        f"head -c {below} /dev/zero | tr '\\000' X", timeout=30)
    assert result.stdout == 'X' * below and result.exit_code == 0

    def observe_output(chunk):
        nonlocal received
        received += len(chunk.encode())

    command = (f"printf 'once\\n' >> {shlex.quote(marker)}; "
               f"head -c {limit} /dev/zero | tr '\\000' X")
    handle = await sandbox.commands.run(command, background=True, timeout=30, on_stdout=observe_output)
    error_name = None
    try:
        await handle.wait()
    except Exception as error:
        if not async_stream_failure(error):
            raise
        error_name = type(error).__name__
    finally:
        await handle.kill()
    assert error_name is not None, 'oversized command stream reported success'
    assert received > 0, 'command failed before any output'
    assert received < limit, 'raw stdout already exceeds the encoded body budget'
    assert await sandbox.files.read(marker) == 'once\n', 'command Start was replayed'
    await healthy(sandbox)
    return {'pid': handle.pid, 'stream_error': error_name,
            'stdout_bytes': received, 'below_limit_stdout_bytes': below, 'observed_starts': 1}


async def disconnect_and_reconnect(sandbox, directory):
    marker = f'{directory}/disconnect-starts'
    gate = f'{directory}/release.fifo'
    ready = asyncio.Event()
    output = ''

    def on_stdout(chunk):
        nonlocal output
        output += chunk
        if 'ready\n' in output:
            ready.set()

    command = (f"mkfifo {shlex.quote(gate)}; printf 'once\\n' >> {shlex.quote(marker)}; "
               f"printf 'ready\\n'; read -r release < {shlex.quote(gate)}; printf 'done\\n'")
    handle = await sandbox.commands.run(command, background=True, timeout=30, on_stdout=on_stdout)
    try:
        await asyncio.wait_for(ready.wait(), timeout=15)
        await handle.disconnect()
        resumed = await sandbox.commands.connect(handle.pid, timeout=30)
        assert resumed.pid == handle.pid
        await sandbox.commands.run(f"printf 'release\\n' > {shlex.quote(gate)}", timeout=15)
        assert 'done\n' in (await resumed.wait()).stdout
        assert await sandbox.files.read(marker) == 'once\n', 'disconnect replayed Start'
        await healthy(sandbox)
        return {'pid': handle.pid, 'reconnected_pid': resumed.pid, 'observed_starts': 1}
    finally:
        await handle.kill()


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    sandbox = await AsyncSandbox.create(template='base', timeout=300, allow_internet_access=False, **owner)
    try:
        directory = f'/tmp/sandproof-async-stream-{uuid.uuid4().hex}'
        await sandbox.files.make_dir(directory)
        report = {'sandbox_id': sandbox.sandbox_id, 'sdk_version': '2.51.0',
                  'configured_budgets': {'file_upload_bytes': args.file_upload_bytes,
                                         'file_download_bytes': args.file_download_bytes,
                                         'command_output_bytes': args.command_output_bytes}}
        report['upload_boundaries'] = await upload_boundary(sandbox, directory, args.file_upload_bytes)
        report['download_boundaries'] = await download_boundary(sandbox, directory, args.file_download_bytes)
        report['command_overflow'] = await command_overflow(sandbox, directory, args.command_output_bytes)
        report['disconnect'] = await disconnect_and_reconnect(sandbox, directory)
    finally:
        assert await AsyncSandbox.kill(sandbox.sandbox_id, **owner), 'test sandbox destruction was not confirmed'
    report['scope'] = 'real async SDK boundaries, output failure and known-PID reconnect; host residuals and remaining concurrency/timeout/revocation cases require separate evidence'
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    def positive_bytes(value):
        size = int(value)
        if size <= 0:
            raise argparse.ArgumentTypeError('byte budget must be positive')
        return size

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--file-upload-bytes', required=True, type=positive_bytes)
    parser.add_argument('--file-download-bytes', required=True, type=positive_bytes)
    parser.add_argument('--command-output-bytes', required=True, type=positive_bytes)
    asyncio.run(exercise(parser.parse_args()))
