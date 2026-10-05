"""Original sync/async SDK command identity and cwd failures on real VMs.

Requires the pinned base with root/ubuntu and private root disks. Every rejected
Start must leave its unique marker absent, followed by a valid same-VM command.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import importlib.metadata
import json
import os
import shlex
import uuid

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import AuthenticationException, InvalidArgumentException

from sdk_command_input import call
from sdk_server_smoke import options


async def healthy_context(sandbox, directory, user, background=False):
    command = "pwd; id -un; printf '%s\\n' \"$SP_ARGUMENT_VALUE\""
    result = await call(sandbox.commands.run, command, cwd=directory, user=user,
                        envs={'SP_ARGUMENT_VALUE': 'spaces and 中文'},
                        background=background, timeout=15)
    if background:
        handle = result
        try:
            result = await call(handle.wait)
        finally:
            await call(handle.disconnect)
    assert (result.stdout, result.stderr, result.exit_code) == (
        f'{directory}\n{user}\nspaces and 中文\n', '', 0)


async def argument_cases(sandbox):
    directory = '/tmp/command-arguments-' + uuid.uuid4().hex
    unknown = 'sp_missing_' + uuid.uuid4().hex
    await call(sandbox.commands.run,
               f'mkdir -m 0777 {shlex.quote(directory)}; '
               f'mkdir -m 0700 {shlex.quote(directory + "/private")}; '
               f'! getent passwd {shlex.quote(unknown)}', timeout=15)
    await call(sandbox.files.write, directory + '/file', 'ordinary file')
    cases = [('unknown_user', unknown, directory, AuthenticationException)]
    for user in ('root', 'ubuntu'):
        cases.extend([
            ('missing_cwd', user, directory + '/missing', InvalidArgumentException),
            ('file_cwd', user, directory + '/file', InvalidArgumentException),
        ])
    cases.append(('denied_cwd', 'ubuntu', directory + '/private', InvalidArgumentException))
    # The same existing private directory is a valid cwd for root. This checks
    # that the permission case is actually user-dependent, not a missing path.
    await healthy_context(sandbox, directory + '/private', 'root')
    results = []
    for background in (False, True):
        for user in ('root', 'ubuntu'):
            await healthy_context(sandbox, directory, user, background)
        for index, (name, user, cwd, expected) in enumerate(cases):
            marker = f'{directory}/rejected-{background}-{index}'
            try:
                result = await call(sandbox.commands.run,
                                    "printf 'unexpected\\n' >> " + shlex.quote(marker),
                                    user=user, cwd=cwd, background=background, timeout=15)
            except Exception as error:
                assert type(error) is expected, (name, type(error).__name__, repr(error))
            else:
                if background:
                    await call(result.kill)
                    await call(result.disconnect)
                raise AssertionError(f'{name} Start unexpectedly succeeded')
            assert not await call(sandbox.files.exists, marker), name
            await healthy_context(sandbox, directory, 'ubuntu')
            results.append(dict(case=name, user='unknown' if user == unknown else user,
                                background=background, exception=expected.__name__,
                                marker_absent=True, subsequent_command_healthy=True))
    assert await call(sandbox.files.read, directory + '/file') == 'ordinary file'
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    report = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template='base', timeout=180,
                             allow_internet_access=False, **owner)
        try:
            report.append(dict(sdk=cls.__name__, cases=await argument_cases(sandbox)))
        finally:
            _kill_confirmed = (await call(sandbox.kill))
            assert _kill_confirmed
    print(json.dumps(dict(sdk_version='2.51.0', results=report,
                          scope='real foreground/background user/cwd failures and valid '
                                'cwd/env/user controls; no fallback or Start marker; '
                                'not every malformed argument or timeout combination'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
