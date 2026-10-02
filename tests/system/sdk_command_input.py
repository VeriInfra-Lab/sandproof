"""Original sync/async SDK stdin and same-PID reconnect on a real fixed base.

Requires the pinned SDK, envd supporting CloseStdin, root and ubuntu users.
Only creates and destroys its own instances; no cloud calls or host writes.
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
import inspect
import json
import os
import shlex
import uuid

from e2b import AsyncSandbox, Sandbox
from sdk_server_smoke import options


async def call(function, *args, **kwargs):
    if inspect.iscoroutinefunction(function):
        return await function(*args, **kwargs)
    result = await asyncio.to_thread(function, *args, **kwargs)
    return await result if inspect.isawaitable(result) else result


READ_INPUT = (
    "import sys,hashlib; data=sys.stdin.buffer.read(); "
    "print(str(len(data))+' '+hashlib.sha256(data).hexdigest())"
)
EMPTY = '0 ' + hashlib.sha256(b'').hexdigest() + '\n'


async def user_cases(sandbox, user):
    command = 'python3 -c ' + shlex.quote(READ_INPUT)
    results = []
    for mode, kwargs in [('omitted', {}), ('false', {'stdin': False})]:
        result = await call(sandbox.commands.run, command, user=user, timeout=15, **kwargs)
        assert (result.stdout, result.stderr, result.exit_code) == (EMPTY, '', 0)
        results.append({'user': user, 'stdin': mode, 'received_bytes': 0})
    for reconnect in (False, True):
        marker = '/tmp/stdin-starts-' + uuid.uuid4().hex
        handle = await call(sandbox.commands.run,
                            "printf 'once\\n' >> " + shlex.quote(marker) + '; exec ' + command,
                            user=user, stdin=True, background=True, timeout=30)
        resumed = None
        complete = False
        payload = b''
        try:
            if reconnect:
                text = 'first line and 中文\n'
                binary = bytes(range(256)) * 257
                await call(handle.send_stdin, text, request_timeout=15)
                await call(handle.disconnect)
                resumed = await call(sandbox.commands.connect, handle.pid, timeout=30)
                assert resumed.pid == handle.pid
                await call(sandbox.commands.send_stdin, handle.pid, binary, request_timeout=15)
                payload = text.encode() + binary
            active = resumed or handle
            await call(active.close_stdin, request_timeout=15)
            result = await call(active.wait)
            digest = hashlib.sha256(payload).hexdigest()
            assert (result.stdout, result.stderr, result.exit_code) == (f'{len(payload)} {digest}\n', '', 0)
            assert await call(sandbox.files.read, marker, user=user) == 'once\n'
            complete = True
            results.append({'user': user, 'stdin': 'enabled', 'reconnected': reconnect,
                            'received_bytes': len(payload), 'sha256': digest, 'start_markers': 1})
        finally:
            if not complete:
                await call(handle.kill)
            for current in (resumed, handle):
                if current is not None:
                    await call(current.disconnect)
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=300,
                             allow_internet_access=False, **owner)
        try:
            for user in ('root', 'ubuntu'):
                cases = await user_cases(sandbox, user)
                for case in cases:
                    case['client'] = cls.__name__
                results.extend(cases)
            assert (await call(sandbox.commands.run, 'printf stdin-healthy')).stdout == 'stdin-healthy'
        finally:
            _kill_confirmed = (await call(sandbox.kill))
            assert _kill_confirmed
    print(json.dumps({'sdk_version': '2.51.0', 'cases': results,
                      'scope': 'real stdin defaults, empty EOF, text/binary content and known-PID reconnect; input budget and timeout matrices are separate'}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
