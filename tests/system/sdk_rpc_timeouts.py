"""Original SDK unary command/filesystem RPC request timeout values on a real VM.

A negative SDK timeout becomes 1 ms, so a fast valid response can beat it.
A timeout never proves a mutating request did not execute; it is not replayed.
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
from e2b.exceptions import InvalidArgumentException, TimeoutException
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_command_input import call
from sdk_server_smoke import options


VALUES = [('omitted', None), ('zero', 0), ('negative', -1), ('above_rpc_limit', 10_000_000)]
INPUT = b'rpc-once\n'
CONTENT = 'rpc-file-integrity\n'
QUIET = "import sys; print(sys.stdin.buffer.read().hex())"


async def invoke(function, case, value, *args):
    kwargs = {} if case == 'omitted' else {'request_timeout': value}
    try:
        result = await call(function, *args, **kwargs)
    except Exception as error:
        if case == 'above_rpc_limit':
            assert type(error) is InvalidArgumentException and 'has >10 digits' in str(error)
            return 'invalid_rpc_timeout', None
        assert case == 'negative' and type(error) is TimeoutException
        assert isinstance(error.__context__, ConnectError)
        assert error.__context__.code is Code.DEADLINE_EXCEEDED
        return 'deadline', None
    else:
        assert case != 'above_rpc_limit', 'invalid RPC header was accepted'
        return 'completed', result


async def file_case(sandbox, operation, case, value):
    directory = '/tmp/rpc-timeout-' + uuid.uuid4().hex
    source, target = directory + '/source', directory + '/target'
    await call(sandbox.files.write, source, CONTENT)
    arguments = {'list': (directory,), 'exists': (source,), 'get_info': (source,),
                 'make_dir': (target,), 'remove': (source,), 'rename': (source, target)}
    outcome, result = await invoke(getattr(sandbox.files, operation), case, value, *arguments[operation])
    if outcome == 'completed':
        if operation in ('exists', 'make_dir'):
            assert result is True
        elif operation == 'list':
            assert [(entry.path, entry.size) for entry in result] == [(source, len(CONTENT))]
        elif operation in ('get_info', 'rename'):
            assert result.path == (target if operation == 'rename' else source)
            assert result.size == len(CONTENT)
        else:
            assert result is None
    # One read-only directory listing observes the result without resubmitting
    # a mutation. A deadline remains unknown even if this observation is empty.
    entries = await call(sandbox.files.list, directory)
    observed = sorted(entry.name for entry in entries)
    assert set(observed) <= {'source', 'target'}
    if outcome == 'invalid_rpc_timeout':
        assert observed == ['source'], 'header rejection changed the filesystem'
        assert await call(sandbox.files.read, source) == CONTENT
    elif outcome == 'completed':
        expected = {'make_dir': ['source', 'target'], 'remove': [], 'rename': ['target']}
        assert observed == expected.get(operation, ['source'])
        if operation != 'remove':
            assert await call(sandbox.files.read, target if operation == 'rename' else source) == CONTENT
    control = directory + '/control'
    await call(sandbox.files.write, control, CONTENT)
    assert await call(sandbox.files.read, control) == CONTENT
    return dict(operation='files.' + operation, case=case, outcome=outcome,
                observed_entries=observed)


async def killed_result(handle):
    try:
        await call(handle.wait)
    except CommandExitException as error:
        assert (error.exit_code, error.error) == (-1, 'signal: killed')
    else:
        raise AssertionError('SendSignal did not terminate the waiting process')


async def command_case(sandbox, operation, case, value):
    marker = '/tmp/rpc-start-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker)
    command += '; exec python3 -c ' + shlex.quote(QUIET)
    handle = await call(sandbox.commands.run, command, stdin=True, background=True, timeout=60)
    try:
        # Ensure the shell side effect and the known process exist before the
        # unary operation (including kill), without a timing-based sleep.
        assert (await call(sandbox.commands.run,
                           'while [ ! -s ' + shlex.quote(marker) + ' ]; do :; done', timeout=15)).exit_code == 0
        if operation == 'list':
            function, arguments = sandbox.commands.list, ()
        elif operation == 'kill':
            function, arguments = sandbox.commands.kill, (handle.pid,)
        elif operation.startswith('handle.'):
            name = operation.split('.')[1]
            function = getattr(handle, name)
            arguments = (INPUT,) if name == 'send_stdin' else ()
        else:
            function = getattr(sandbox.commands, operation)
            arguments = (handle.pid, INPUT) if operation == 'send_stdin' else (handle.pid,)
        outcome, result = await invoke(function, case, value, *arguments)
        if outcome == 'completed':
            if operation == 'list':
                assert handle.pid in [entry.pid for entry in result]
            elif operation == 'kill':
                assert result is True
            else:
                assert result is None
        live = handle.pid in [entry.pid for entry in await call(sandbox.commands.list)]
        received = None
        if operation in ('send_stdin', 'handle.send_stdin', 'list'):
            assert live
            await call(handle.close_stdin)
            finished = await call(handle.wait)
            received = bytes.fromhex(finished.stdout.strip())
            assert finished.stderr == '' and finished.exit_code == 0
            if operation == 'list' or outcome == 'invalid_rpc_timeout':
                assert received == b''
            elif outcome == 'completed':
                assert received == INPUT
            else:
                assert received in (b'', INPUT), 'unknown input was duplicated'
        elif outcome == 'invalid_rpc_timeout':
            assert live, 'rejected control request affected the process'
        elif outcome == 'completed':
            if operation == 'kill':
                await killed_result(handle)
            else:
                finished = await call(handle.wait)
                assert (finished.stdout, finished.stderr, finished.exit_code) == ('\n', '', 0)
        # With a deadline on kill/CloseStdin, only record the observed state.
        # The finally block explicitly cleans up the known PID; no RPC replay
        # is used to turn an unknown outcome into a claimed original success.
        assert await call(sandbox.files.read, marker) == 'once\n'
        return dict(operation=operation if operation.startswith('handle.') else 'commands.' + operation,
                    case=case, outcome=outcome, observed_live=live,
                    received_bytes=None if received is None else len(received), start_markers=1)
    finally:
        await call(handle.kill)
        await call(handle.disconnect)
        assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=300,
                             allow_internet_access=False, **owner)
        try:
            rows = []
            for operation in ('get_info', 'exists', 'list', 'make_dir', 'remove', 'rename'):
                for case, value in VALUES:
                    rows.append(await file_case(sandbox, operation, case, value))
            for operation in ('list', 'kill', 'send_stdin', 'close_stdin',
                              'handle.send_stdin', 'handle.close_stdin'):
                for case, value in VALUES:
                    rows.append(await command_case(sandbox, operation, case, value))
            results.append(dict(sdk=cls.__name__, cases=rows))
        finally:
            assert await call(sandbox.kill)
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='selected unary filesystem and command request timeout values; '
                                'negative 1 ms may race success; not lifecycle or watcher API'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
