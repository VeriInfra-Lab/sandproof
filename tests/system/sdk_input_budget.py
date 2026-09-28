"""Real original-SDK stdin request boundaries, integrity and failure recovery.

The configured budget counts the encoded Process RPC body, not raw stdin bytes.
Requires an isolated server with the explicit command input budget below.
"""

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
import shlex
import uuid

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import AsyncSandbox, Sandbox
from e2b.envd.client_shared import ENVD_JSON_CODEC
from e2b.envd.process import process_pb
from e2b.exceptions import TimeoutException
from protobuf import Oneof

from sdk_command_input import READ_INPUT, call
from sdk_server_smoke import options


def encoded_size(pid, size):
    # Measure the pinned SDK's actual unary JSON representation. Calls below
    # still use the original public SDK, without changing its transport.
    message = process_pb.SendInputRequest(
        process=process_pb.ProcessSelector(selector=Oneof('pid', pid)),
        input=process_pb.ProcessInput(input=Oneof('stdin', b'X' * size)),
    )
    return len(ENVD_JSON_CODEC.encode(message))


def largest_fitting(pid, budget):
    low, high = 0, budget
    assert encoded_size(pid, low) <= budget < encoded_size(pid, high)
    while high - low > 1:
        middle = (low + high) // 2
        if encoded_size(pid, middle) <= budget:
            low = middle
        else:
            high = middle
    return low


def is_gateway_input_failure(error):
    # Current Process request limits close the upstream body and return 502.
    # The pinned SDK maps that exact plain HTTP error to TimeoutException;
    # an ordinary client deadline is not evidence of limit enforcement.
    context = error.__context__
    return (type(error) is TimeoutException
            and isinstance(context, ConnectError)
            and context.code is Code.UNAVAILABLE
            and context.message == 'HTTP 502'
            and context.__cause__ is None)


async def stdin_boundary(sandbox, budget):
    marker = '/tmp/stdin-budget-' + uuid.uuid4().hex
    command = "printf 'once\\n' >> " + shlex.quote(marker)
    command += '; exec python3 -c ' + shlex.quote(READ_INPUT)
    handle = await call(sandbox.commands.run, command, stdin=True,
                        background=True, timeout=60)
    finished = False
    try:
        fitting = largest_fitting(handle.pid, budget)
        assert fitting > 0
        before = encoded_size(handle.pid, fitting)
        after = encoded_size(handle.pid, fitting + 1)
        assert before <= budget < after
        await call(handle.send_stdin, b'', request_timeout=15)
        first = b'A' * fitting
        await call(handle.send_stdin, first, request_timeout=15)
        try:
            await call(sandbox.commands.send_stdin, handle.pid,
                       b'X' * (fitting + 1), request_timeout=15)
        except Exception as error:
            assert is_gateway_input_failure(error), (
                'unexpected oversized SendInput failure', type(error).__name__,
                repr(error), repr(error.__context__))
            failure = type(error).__name__
        else:
            raise AssertionError('oversized SendInput returned success')
        assert (await call(sandbox.commands.run, 'printf input-budget-healthy',
                           timeout=15)).stdout == 'input-budget-healthy'
        second = b'B' * fitting
        await call(sandbox.commands.send_stdin, handle.pid, second, request_timeout=15)
        await call(handle.close_stdin, request_timeout=15)
        result = await call(handle.wait)
        expected = first + second
        digest = hashlib.sha256(expected).hexdigest()
        assert (result.stdout, result.stderr, result.exit_code) == (
            f'{len(expected)} {digest}\n', '', 0)
        assert await call(sandbox.files.read, marker) == 'once\n'
        finished = True
        return dict(pid=handle.pid, budget_bytes=budget, fitting_stdin_bytes=fitting,
                    fitting_body_bytes=before, overflowing_body_bytes=after,
                    rejected_error=failure, received_bytes=len(expected),
                    sha256=digest, start_markers=1)
    finally:
        if not finished:
            await call(handle.kill)
        await call(handle.disconnect)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert 1024 <= args.command_input_bytes <= 8 * 1024**2
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=180,
                             allow_internet_access=False, **owner)
        try:
            results.append(dict(sdk=cls.__name__,
                                **await stdin_boundary(sandbox, args.command_input_bytes)))
        finally:
            assert await call(sandbox.kill)
    print(json.dumps(dict(sdk_version='2.51.0', cases=results,
                          scope='selected per-request SendInput boundaries and integrity; '
                                'base64 length granularity, not every encoded byte; '
                                'not a cumulative per-command input quota or timeout matrix'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--command-input-bytes', type=int, default=1024**2)
    asyncio.run(exercise(parser.parse_args()))
