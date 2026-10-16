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
from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_server_smoke import options
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate


async def call(function, *args, **kwargs):
    if inspect.iscoroutinefunction(function):
        return await function(*args, **kwargs)
    result = await asyncio.to_thread(function, *args, **kwargs)
    return await result if inspect.isawaitable(result) else result


async def destroy_probes(live):
    """Try every known instance once, retaining all failures for private logs.

    A failure for the control VM must not prevent teardown of the victim VM.
    This is an SDK acknowledgement, not the supervisor's physical resource audit.
    """
    errors = []
    for sandbox in reversed(live):
        try:
            confirmed = await call(sandbox.kill)
            if confirmed is not True:
                raise RuntimeError(f'destruction not confirmed for {sandbox.sandbox_id}')
        except BaseException as error:
            errors.append(error)
    if errors:
        raise BaseExceptionGroup('known probe instance cleanup failed', errors)


def public_pid(pid):
    assert type(pid) is int and 0 < pid <= 4194304, 'invalid original command PID'
    return pid


class KnownCommand:
    """One returned Start identity; subscription close and process End are distinct."""
    def __init__(self, commands, original):
        self.commands, self.original = commands, original
        self.handles, self.closed = [original], set()
        self.pid, self.finished, self.signal_attempted = None, False, False

    async def __aenter__(self):
        try:
            self.pid = public_pid(self.original.pid)
        except BaseException as error:
            await self.cleanup(error)
            raise
        return self

    async def __aexit__(self, _kind, error, _traceback):
        await self.cleanup(error)
        return False

    def add(self, handle):
        # Retain even a wrong-PID reply so its subscription can be closed.
        if all(handle is not known for known in self.handles):
            self.handles.append(handle)
        assert public_pid(handle.pid) == self.pid, 'reconnect changed original PID'
        return handle

    async def wait(self, handle):
        assert any(handle is known for known in self.handles)
        assert public_pid(handle.pid) == self.pid
        try:
            result = await call(handle.wait)
        except CommandExitException:
            self.finished = True
            raise
        self.finished = True
        return result

    async def disconnect(self, handle):
        assert any(handle is known for known in self.handles)
        if id(handle) not in self.closed:
            self.closed.add(id(handle))  # Unknown replies never cause a retry.
            await call(handle.disconnect)

    async def kill(self, **kwargs):
        assert self.pid is not None and not self.finished, 'no unfinished original command'
        assert not self.signal_attempted, 'original signal was already attempted'
        self.signal_attempted = True
        return await call(self.commands.kill, self.pid, **kwargs)

    async def cleanup(self, primary):
        errors = []
        if self.pid is not None and not self.finished and not self.signal_attempted:
            try:
                await self.kill(request_timeout=15)
            except BaseException as error:
                errors.append(error)
        for handle in reversed(self.handles):
            try:
                await self.disconnect(handle)
            except BaseException as error:
                errors.append(error)
        if errors:
            raise BaseExceptionGroup('known command and subscription cleanup failed',
                                     ([primary] if primary is not None else []) + errors)


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
        payload = b''
        async with KnownCommand(sandbox.commands, handle) as owned:
            active = handle
            if reconnect:
                text = 'first line and 中文\n'
                binary = bytes(range(256)) * 257
                await call(handle.send_stdin, text, request_timeout=15)
                await owned.disconnect(handle)
                active = owned.add(await call(sandbox.commands.connect, owned.pid, timeout=30))
                await call(sandbox.commands.send_stdin, owned.pid, binary, request_timeout=15)
                payload = text.encode() + binary
            await call(active.close_stdin, request_timeout=15)
            result = await owned.wait(active)
            digest = hashlib.sha256(payload).hexdigest()
            assert (result.stdout, result.stderr, result.exit_code, result.error) == (f'{len(payload)} {digest}\n', '', 0, None)
            started = await call(sandbox.files.read, marker, user=user)
            assert started == 'once\n'
            results.append({'user': user, 'stdin': 'enabled', 'reconnected': reconnect,
                            'received_bytes': len(payload), 'sha256': digest, 'start_markers': 1})
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, candidate = guest_candidate(args)
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    observations = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template=template, timeout=300,
                             allow_internet_access=False, **owner)
        try:
            before = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            for user in ('root', 'ubuntu'):
                cases = await user_cases(sandbox, user)
                for case in cases:
                    case['client'] = cls.__name__
                results.extend(cases)
            assert (await call(sandbox.commands.run, 'printf stdin-healthy')).stdout == 'stdin-healthy'
            after = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            assert before == after, 'Rust guest identity changed during stdin cases'
            if candidate is not None:
                observations.append(dict(sdk=cls.__name__, sandbox_id=sandbox.sandbox_id, rust_guest=before))
        finally:
            await destroy_probes([sandbox])
    report = {'sdk_version': '2.51.0', 'cases': results,
              'scope': 'real stdin defaults, empty EOF, text/binary content and known-PID reconnect; input budget and timeout matrices are separate'}
    if getattr(args, 'template', None) is not None:
        report['template'] = template
    if candidate is not None:
        report['guests'] = observations
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    asyncio.run(exercise(parser.parse_args()))
