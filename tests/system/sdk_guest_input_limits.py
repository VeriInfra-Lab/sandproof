"""Large original SDK calls with a pinned, unchanged internal write chunk.

The chunk is not a public per-call limit. Complete request decoding stays under
the original HTTP body/admission budgets; no larger test configuration is used.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import asyncio
import hashlib
import importlib.metadata
import json
import os
import shlex

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import RateLimitException
from sdk_command_input import KnownCommand, READ_INPUT, call, destroy_probes
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_server_smoke import options

BODY_LIMIT = 'command request exceeds configured body limits'


def configured_limit(args):
    # guest_candidate validates the release and all four pins before this read.
    from tools import guest_artifacts
    info, data = guest_artifacts.fingerprint(_GuardPath(args.guest_configuration), 1024**2, content=True)
    assert info['sha256'] == args.guest_configuration_sha256
    value = json.loads(data)['commands']['owners']['input_chunk_bytes']
    assert type(value) is int and 0 < value <= 65536, 'bounded input-chunk fixture required'
    return value


async def rejected(function, *arguments, messages):
    try:
        await call(function, *arguments, request_timeout=15)
    except RateLimitException as error:
        context = error.__context__
        assert type(error) is RateLimitException and isinstance(context, ConnectError)
        assert context.code is Code.RESOURCE_EXHAUSTED and context.message in messages
        assert str(error) == context.message + ': Rate limit exceeded, please try again later.'
        return context.message
    raise AssertionError('oversized input was not explicitly rejected')


async def user_case(sandbox, user, limit):
    handle = await call(sandbox.commands.run, '/usr/bin/python3 -c ' + shlex.quote(READ_INPUT),
                        user=user, stdin=True, background=True, timeout=30)
    accepted = bytearray()
    async with KnownCommand(sandbox.commands, handle) as owned:
        fitting = b'A' * limit
        await call(handle.send_stdin, fitting, request_timeout=15)
        accepted.extend(fitting)
        over_chunk = b'X' * (limit + 1)
        await call(sandbox.commands.send_stdin, owned.pid, over_chunk, request_timeout=15)
        accepted.extend(over_chunk)
        large = bytes(range(256)) * 257
        await call(sandbox.commands.send_stdin, owned.pid, large, request_timeout=15)
        accepted.extend(large)
        # Retain the original multi-call integrity check as well. Both forms
        # must deliver the same bytes, once, under the same original PID.
        for start in range(0, len(large), limit):
            chunk = large[start:start + limit]
            await call(sandbox.commands.send_stdin, owned.pid, chunk, request_timeout=15)
            accepted.extend(chunk)
        assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'
        await call(handle.close_stdin, request_timeout=15)
        result = await owned.wait(handle)
        digest = hashlib.sha256(accepted).hexdigest()
        assert (result.stdout, result.stderr, result.exit_code) == (f'{len(accepted)} {digest}\n', '', 0)
    return dict(user=user, input_chunk_bytes=limit, single_call_bytes=len(large),
                delivered_bytes=len(accepted), sha256=digest)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, candidate = guest_candidate(args)
    assert candidate is not None, 'explicit pinned Rust candidate required'
    limit = configured_limit(args)
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    rows = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template=template, timeout=300,
                             allow_internet_access=False, **owner)
        try:
            before = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            for user in ('root', 'ubuntu'):
                rows.append(dict(sdk=cls.__name__, **await user_case(sandbox, user, limit)))
            after = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            assert before == after, 'guest identity changed during input-limit checks'
        finally:
            await destroy_probes([sandbox])
    print(json.dumps(dict(sdk_version='2.51.0', cases=rows,
        scope='original large SDK calls plus multi-call integrity; original request budgets and internal chunk unchanged'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser, required=True)
    asyncio.run(exercise(parser.parse_args()))
