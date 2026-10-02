"""Pinned SDK file users, path errors and partial batches on real private VMs.

The fixed envd handles file operations as root: user selects home/ownership,
not a guest DAC boundary. The caller supplies only a disposable host canary
path; the KVM harness must independently verify its content stays unchanged.
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
from io import BytesIO
import json
import os
from pathlib import PurePosixPath
import shlex
import time
import uuid

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import AuthenticationException, FileNotFoundException, InvalidArgumentException
from e2b.sandbox.filesystem.filesystem import FileType

from sdk_command_input import call
from sdk_server_smoke import options


async def rejected(expected, function, *args, **kwargs):
    try:
        await call(function, *args, **kwargs)
    except Exception as error:
        assert type(error) is expected, (function.__name__, type(error).__name__, str(error))
    else:
        raise AssertionError(f'{function.__name__} unexpectedly succeeded')


async def contents(sandbox, path, expected):
    assert await call(sandbox.files.read, path, format='bytes') == expected
    info = await call(sandbox.files.get_info, path)
    assert info.type == FileType.FILE and info.size == len(expected)
    return hashlib.sha256(expected).hexdigest()


async def healthy(sandbox, directory):
    path = directory + '/healthy'
    await call(sandbox.files.write, path, 'healthy', user='ubuntu')
    assert await call(sandbox.files.read, path, user='ubuntu') == 'healthy'
    assert (await call(sandbox.commands.run, 'id -un', user='ubuntu')).stdout == 'ubuntu\n'


async def user_cases(sandbox, directory):
    private = directory + '/private'
    secret = private + '/secret'
    await call(sandbox.commands.run,
               f'mkdir -m 0700 {shlex.quote(private)}; '
               f'printf original > {shlex.quote(secret)}; chmod 0600 {shlex.quote(secret)}')
    # The command really runs as ubuntu and cannot traverse the root directory.
    denied = await call(sandbox.commands.run,
                        f'if test -r {shlex.quote(secret)}; then exit 41; fi; id -un', user='ubuntu')
    assert denied.stdout == 'ubuntu\n'
    # File APIs intentionally retain the pinned envd root behavior.
    assert await call(sandbox.files.read, secret, user='ubuntu') == 'original'
    info = await call(sandbox.files.get_info, secret, user='ubuntu')
    assert info.owner == 'root' and info.mode & 0o777 == 0o600
    assert secret in {entry.path for entry in await call(sandbox.files.list, private, user='ubuntu')}
    await call(sandbox.files.write, secret, 'overwritten', user='ubuntu')
    info = await call(sandbox.files.get_info, secret)
    assert info.owner == 'ubuntu' and info.mode & 0o777 == 0o600
    assert await call(sandbox.files.read, secret) == 'overwritten'
    assert await call(sandbox.files.make_dir, private + '/child', user='ubuntu')
    assert (await call(sandbox.files.get_info, private + '/child')).owner == 'ubuntu'
    assert (await call(sandbox.files.get_info, private)).owner == 'root'
    denied = await call(sandbox.commands.run,
                        f'if test -r {shlex.quote(secret)}; then exit 42; fi; id -un', user='ubuntu')
    assert denied.stdout == 'ubuntu\n'
    results = [dict(case='file_user_is_not_guest_dac', command_denied=True,
                    file_read_list_stat_succeeded=True, overwrite_owner='ubuntu', mode='0600')]
    for user, home in [('root', '/root'), ('ubuntu', '/home/ubuntu')]:
        relative = 'file-home-' + uuid.uuid4().hex + '/nested/item'
        for path in (relative, '~/' + relative):
            written = await call(sandbox.files.write, path, 'home:' + user, user=user)
            assert written.path == home + '/' + relative
            assert await call(sandbox.files.read, path, user=user) == 'home:' + user
            info = await call(sandbox.files.get_info, path, user=user)
            assert info.path == written.path and info.owner == user
            results.append(dict(case='relative_home', user=user, tilde=path.startswith('~')))
    unknown = 'sp_missing_' + uuid.uuid4().hex
    await call(sandbox.commands.run, '! getent passwd ' + shlex.quote(unknown))
    absent = directory + '/unknown-user'
    operations = [('read', (secret,)), ('write', (absent, 'unexpected')),
                  ('get_info', (secret,)), ('list', (private,)),
                  ('make_dir', (absent,)), ('exists', (secret,))]
    for name, args in operations:
        await rejected(AuthenticationException, getattr(sandbox.files, name), *args, user=unknown)
        assert not await call(sandbox.files.exists, absent)
        await healthy(sandbox, directory)
        results.append(dict(case='unknown_user', operation=name, exception='AuthenticationException',
                            no_created_path=True, subsequent_call_healthy=True))
    return results


async def path_cases(sandbox, directory):
    existing = directory + '/ordinary'
    await call(sandbox.files.write, existing, 'unchanged')
    absent = directory + '/absent'
    cases = [('read', absent, FileNotFoundException),
             ('get_info', absent, FileNotFoundException),
             ('list', absent, FileNotFoundException),
             ('read', directory, InvalidArgumentException),
             ('list', existing, InvalidArgumentException),
             ('make_dir', existing, InvalidArgumentException)]
    results = []
    for user in ('root', 'ubuntu'):
        assert not await call(sandbox.files.exists, absent, user=user)
        for name, path, expected in cases:
            await rejected(expected, getattr(sandbox.files, name), path, user=user)
            await healthy(sandbox, directory)
            results.append(dict(case='path_error', operation=name, user=user,
                                exception=expected.__name__, subsequent_call_healthy=True))
        for octet in (False, True):
            await rejected(InvalidArgumentException, sandbox.files.write, directory, 'unexpected',
                           user=user, use_octet_stream=octet)
            results.append(dict(case='write_directory', user=user, octet=octet,
                                exception='InvalidArgumentException'))
        await rejected(InvalidArgumentException, sandbox.files.write, '~somebody/item', 'unexpected', user=user)
        results.append(dict(case='unsupported_tilde_user', user=user, exception='InvalidArgumentException'))
    assert await call(sandbox.files.read, existing) == 'unchanged'
    return results


async def batch_cases(sandbox, directory):
    results = []
    for octet in (False, True):
        root = directory + ('/octet' if octet else '/multipart')
        bad = root + '/directory'
        await call(sandbox.files.make_dir, bad)
        first, last = root + '/first', root + '/last'
        data = b'complete before or alongside the failed item\n'
        entries = [dict(path=path, data=BytesIO(data) if octet else data.decode())
                   for path in (first, bad, last)]
        await rejected(InvalidArgumentException, sandbox.files.write_files, entries,
                       user='ubuntu', use_octet_stream=octet)
        parallel = isinstance(sandbox, AsyncSandbox) and octet
        # gather propagates an error before its other uploads have necessarily
        # completed. Await observable completion, without imposing their order.
        deadline = time.monotonic() + 15
        while True:
            first_done = await call(sandbox.files.exists, first)
            if first_done:
                first_done = (await call(sandbox.files.get_info, first)).size == len(data)
            last_done = await call(sandbox.files.exists, last)
            if last_done and parallel:
                last_done = (await call(sandbox.files.get_info, last)).size == len(data)
            if first_done and (last_done or not parallel):
                break
            assert time.monotonic() < deadline, 'batch side effects did not settle'
            await asyncio.sleep(0.02)
        digest = await contents(sandbox, first, data)
        if parallel:
            assert await contents(sandbox, last, data) == digest
        else:
            assert not last_done, 'sequential batch continued past its error'
        assert (await call(sandbox.files.get_info, bad)).type == FileType.DIR
        await healthy(sandbox, directory)
        results.append(dict(case='partial_batch', octet=octet, first_preserved=True,
                            last_written=parallel, sha256=digest,
                            exception='InvalidArgumentException', subsequent_call_healthy=True))
    return results


async def isolation_cases(victim, control, directory, host_canary):
    # Only a new harness-owned canary path is allowed; never probe real secrets.
    target = PurePosixPath(host_canary)
    assert target.is_absolute() and target.name == 'host-canary'
    assert '..' not in target.parts and str(target).startswith('/var/lib/sp-01a0e1bd/fp8-')
    assert not await call(victim.files.exists, host_canary)
    assert not await call(control.files.exists, host_canary)
    original = b'control VM independent file\n'
    await call(control.files.write, host_canary, BytesIO(original))
    await call(victim.files.make_dir, str(target.parent))
    link = directory + '/guest-root'
    await call(victim.commands.run, 'ln -s / ' + shlex.quote(link))
    spellings = [host_canary, directory + '/../../' + host_canary.lstrip('/'),
                 link + host_canary, '/proc/1/root' + host_canary]
    results = []
    for index, path in enumerate(spellings):
        data = f'guest-only-write-{index}\n'.encode()
        await call(victim.files.write, path, BytesIO(data), user='ubuntu')
        digest = await contents(victim, host_canary, data)
        assert await call(victim.files.read, path, format='bytes') == data
        await contents(control, host_canary, original)
        results.append(dict(case='guest_path_isolation', spelling=index, sha256=digest,
                            control_vm_unchanged=True))
    return results


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        live = []
        try:
            for _ in range(2):
                live.append(await call(cls.create, template='base', timeout=300,
                                       allow_internet_access=False, **owner))
            victim, control = live
            directory = '/tmp/file-permissions-' + uuid.uuid4().hex
            await call(victim.commands.run, 'mkdir -m 0777 ' + shlex.quote(directory))
            cases = await user_cases(victim, directory)
            cases += await path_cases(victim, directory)
            cases += await batch_cases(victim, directory)
            cases += await isolation_cases(victim, control, directory, args.host_canary)
            results.append(dict(sdk=cls.__name__, victim=victim.sandbox_id,
                                control=control.sandbox_id, cases=cases))
        finally:
            for sandbox in reversed(live):
                _kill_confirmed = (await call(sandbox.kill))
                assert _kill_confirmed, 'destruction not confirmed'
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='file user/home/ownership, selected path errors, partial batches, '
                                'private VM path controls; invoking harness must audit host canary '
                                'and exact resources; not guest DAC or exhaustive path coverage'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--host-canary', required=True)
    asyncio.run(exercise(parser.parse_args()))
