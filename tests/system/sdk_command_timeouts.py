"""Original sync/async command stream and opening timeout argument matrix.

Each public run/connect interface receives omitted, zero, negative and a value
above Connect's ten-digit millisecond header limit. Request/opening deadlines
are checked separately from the command stream deadline and sandbox lifetime.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os
import shlex
import uuid

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import AsyncSandbox, Sandbox
from e2b.envd.client_async import _request_timeout_exception
from e2b.envd.rpc import handle_rpc_exception
from e2b.exceptions import InvalidArgumentException, TimeoutException

from sdk_command_input import call
from sdk_server_smoke import options


VALUES = [('omitted', None), ('zero', 0), ('negative', -1), ('above_rpc_limit', 10_000_000)]
DEADLINE_SUFFIX = str(handle_rpc_exception(ConnectError(Code.DEADLINE_EXCEEDED, '')))
QUIET = "import sys; sys.stdin.buffer.read(); print('finished')"


def expected_outcome(asynchronous, field, case):
    if field == 'timeout' and case == 'above_rpc_limit':
        return 'invalid_rpc_timeout'
    if case == 'negative':
        if field == 'timeout':
            return 'stream_deadline'
        if asynchronous:
            return 'opening_deadline'
    return 'completed'


def verify_failure(error, expected):
    if expected == 'invalid_rpc_timeout':
        assert type(error) is InvalidArgumentException and 'has >10 digits' in str(error)
    elif expected == 'opening_deadline':
        assert type(error) is TimeoutException
        assert str(error) == str(_request_timeout_exception(-1))
        assert isinstance(error.__cause__, TimeoutError)
    elif expected == 'stream_deadline':
        assert type(error) is TimeoutException
        context = error.__context__
        if isinstance(context, ConnectError):
            assert context.code is Code.DEADLINE_EXCEEDED
        else:
            # Async handle.wait may lose the original exception chain. Only
            # the pinned DEADLINE_EXCEEDED mapping is valid here, not its
            # UNAVAILABLE/EOF mapping or a generic TimeoutException.
            assert context is None and error.__cause__ is None
            assert str(error).endswith(DEADLINE_SUFFIX)
    else:
        raise AssertionError(f'unexpected failure for {expected}: {error!r}')


def arguments(field, case, value):
    kwargs = {'timeout': 15} if field == 'request_timeout' else {}
    if case != 'omitted':
        kwargs[field] = value
    return kwargs


async def marker_count(sandbox, marker):
    if not await call(sandbox.files.exists, marker):
        return 0
    content = await call(sandbox.files.read, marker)
    assert content == 'once\n', 'a Start side effect was repeated'
    return 1


async def run_case(sandbox, asynchronous, field, case, value, background):
    expected = expected_outcome(asynchronous, field, case)
    marker = '/tmp/run-timeout-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker) + '; '
    if expected == 'stream_deadline':
        # No successful end event is possible before an explicit stdin close.
        # This prevents a quick command from racing the one-millisecond limit.
        command += 'exec python3 -c ' + shlex.quote(QUIET)
    else:
        command += "printf 'finished\\n'"
    handle = None
    try:
        try:
            result = await call(sandbox.commands.run, command, stdin=True,
                                background=background, **arguments(field, case, value))
            if background:
                handle = result
                result = await call(handle.wait)
        except Exception as error:
            verify_failure(error, expected)
        else:
            assert expected == 'completed', 'deadline or malformed header was ignored'
            assert (result.stdout, result.stderr, result.exit_code) == ('finished\n', '', 0)
        count = await marker_count(sandbox, marker)
        if expected == 'completed':
            assert count == 1
        if expected in ('invalid_rpc_timeout', 'opening_deadline'):
            assert count == 0
    finally:
        if handle is not None:
            await call(handle.kill)
            await call(handle.disconnect)
        # An opening stream deadline can lose the PID after Start executed.
        # Discover this exact submitted command without replaying its shell.
        for process in await call(sandbox.commands.list):
            if marker in ' '.join(process.args):
                await call(sandbox.commands.kill, process.pid)
    return dict(operation='run', parameter=field, case=case, background=background,
                outcome=expected, observed_start_markers=count)


async def connect_case(sandbox, asynchronous, field, case, value):
    expected = expected_outcome(asynchronous, field, case)
    marker = '/tmp/connect-timeout-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker)
    command += '; exec python3 -c ' + shlex.quote(QUIET)
    original = await call(sandbox.commands.run, command, stdin=True,
                          background=True, timeout=60)
    connected = None
    recovery = None
    try:
        await call(original.disconnect)
        try:
            connected = await call(sandbox.commands.connect, original.pid,
                                   **arguments(field, case, value))
            assert connected.pid == original.pid
            if expected == 'stream_deadline':
                await call(connected.wait)
        except Exception as error:
            verify_failure(error, expected)
        else:
            assert expected == 'completed', 'invalid connect timeout was ignored'
        assert original.pid in [item.pid for item in await call(sandbox.commands.list)]
        if expected != 'completed':
            if connected is not None:
                await call(connected.disconnect)
            recovery = await call(sandbox.commands.connect, original.pid, timeout=15)
        active = recovery or connected
        assert active.pid == original.pid
        await call(active.close_stdin, request_timeout=15)
        result = await call(active.wait)
        assert (result.stdout, result.stderr, result.exit_code) == ('finished\n', '', 0)
        assert await marker_count(sandbox, marker) == 1
    finally:
        await call(original.kill)
        for handle in (recovery, connected, original):
            if handle is not None:
                await call(handle.disconnect)
    return dict(operation='connect', parameter=field, case=case,
                outcome=expected, same_pid_recovered=True, start_markers=1)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=300,
                             allow_internet_access=False, **owner)
        try:
            rows = []
            for field in ('timeout', 'request_timeout'):
                for case, value in VALUES:
                    for background in (False, True):
                        rows.append(await run_case(sandbox, cls is AsyncSandbox,
                                                   field, case, value, background))
                    rows.append(await connect_case(sandbox, cls is AsyncSandbox,
                                                   field, case, value))
                    assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'
            results.append(dict(sdk=cls.__name__, cases=rows))
        finally:
            assert await call(sandbox.kill)
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='run/connect stream and opening timeout values; wait uses '
                                'the existing stream deadline, no separate timeout argument; '
                                'not file/unary RPC or sandbox lifetime timeout matrices'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
