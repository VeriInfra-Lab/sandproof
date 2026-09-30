"""Original SDK acceptance for a newly built base's private writable root.

Installs a tiny, locally constructed Debian package inside each disposable VM.
No repository, network download or host package manager is used. This checks
package/file isolation, not any Harbor task's dependency initialization.
The supervisor must separately audit the base hash and exact host resources.
"""

import argparse
import asyncio
import importlib.metadata
import json
import os
import shlex

from e2b import AsyncSandbox, Sandbox
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_command_input import call
from sdk_server_smoke import options


PACKAGE = 'sandproof-private-root-probe'
CONFIG = '/etc/sandproof-private-root-probe.conf'
EXECUTABLE = '/usr/local/bin/sandproof-private-root-probe'
STAGING = '/tmp/sandproof-private-root-package'
FAILED_INIT = '/etc/sandproof-failed-initializer'


async def run(sandbox, command):
    # Package/system administration must remain explicit when a selected
    # template has a non-root default. Default-user behavior is tested apart.
    result = await call(sandbox.commands.run, command, user='root', timeout=30)
    assert result.exit_code == 0
    return result.stdout


async def clean(sandbox):
    for path in (CONFIG, EXECUTABLE, STAGING, FAILED_INIT):
        assert not await call(sandbox.files.exists, path), ('unexpected base path', path)
    # dpkg-query can find a removed package's config-only record: a clean VM
    # must not inherit even that record from another instance.
    output = await run(sandbox, f'if dpkg-query -W {PACKAGE} >/dev/null 2>&1; '
                               "then printf present; else printf absent; fi")
    assert output == 'absent'


async def install(sandbox, version, identity):
    control = (f'Package: {PACKAGE}\nVersion: {version}\nArchitecture: all\n'
               'Maintainer: Sandproof acceptance <test@example.invalid>\n'
               'Description: Disposable private-root isolation probe\n')
    await call(sandbox.files.write, STAGING + '/DEBIAN/control', control, user='root')
    await call(sandbox.files.write, STAGING + CONFIG, identity + '\n', user='root')
    await call(sandbox.files.write, STAGING + EXECUTABLE,
               '#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(identity) + '\n', user='root')
    await run(sandbox, f'chmod 0755 {STAGING + EXECUTABLE} && '
                      f'dpkg-deb --build --root-owner-group {STAGING} {STAGING}.deb && '
                      f'dpkg --install {STAGING}.deb')


async def installed(sandbox, version, identity, config=None, *, resources=(1, 512)):
    assert await call(sandbox.files.read, CONFIG) == (config or identity) + '\n'
    assert await run(sandbox, EXECUTABLE) == identity + '\n'
    assert await run(sandbox, f"dpkg-query -W -f='${{Version}} ${{Status}}' {PACKAGE}") == (
        version + ' install ok installed')
    info = await call(sandbox.get_info)
    assert (info.cpu_count, info.memory_mb) == resources


async def task_profile_data(sandbox):
    # Reuse the exact shared data assertions, including original SDK handling
    # of nonzero exit, disconnect/reconnect of one PID, and explicit kill.
    from sdk_data_contract import async_case, sync_case
    rows = []
    for user in ('root', 'ubuntu'):
        result = await call(sandbox.commands.run,
                            'test -w "$HOME" && test "$(id -un)" = ' + user,
                            user=user, timeout=15)
        assert result.exit_code == 0
        directory = '/tmp/sandproof-profile-data-' + user
        if isinstance(sandbox, AsyncSandbox):
            rows.append(await async_case(sandbox, directory, user))
        else:
            rows.append(await call(sync_case, sandbox, directory, user))
    return rows


async def failed_initializer(sandbox):
    # This is a guest initialization failure, not a controller crash. Keep a
    # live child and a partial system write for VM destruction to reclaim.
    try:
        await call(sandbox.commands.run,
                   f'nohup sleep 600 </dev/null >/dev/null 2>&1 & '
                   f'printf "%s\\n" "$!" > {FAILED_INIT}; exit 42', user='root', timeout=15)
    except CommandExitException as error:
        assert error.exit_code == 42
    else:
        raise AssertionError('failed initializer reported success')
    pid = int((await call(sandbox.files.read, FAILED_INIT)).strip())
    assert pid > 1
    await run(sandbox, f'kill -0 {pid}')
    return dict(exit_code=42, partial_write=FAILED_INIT, live_guest_child_pid=pid)


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    live = []
    rows = []
    resources = (args.expected_cpus, args.expected_memory_mib)
    async def check(*positional, **kwargs):
        await installed(*positional, **kwargs, resources=resources)
    try:
        for _ in range(2):
            live.append(await call(cls.create, template='base', timeout=300,
                                   allow_internet_access=False, **owner))
        first, second = live
        await clean(first)
        await clean(second)
        if args.task_profile:
            rows.append(dict(case='root_and_ubuntu_data_contract',
                             results=await task_profile_data(first)))
        await install(first, '1.0', first.sandbox_id)
        await check(first, '1.0', first.sandbox_id)
        await clean(second)
        rows.append(dict(case='first_install_does_not_change_second'))
        await install(second, '2.0', second.sandbox_id)
        await check(second, '2.0', second.sandbox_id)
        await check(first, '1.0', first.sandbox_id)
        rows.append(dict(case='same_paths_and_package_have_independent_contents'))
        changed = first.sandbox_id + '-modified'
        await call(first.files.write, CONFIG, changed + '\n')
        await check(first, '1.0', first.sandbox_id, config=changed)
        await check(second, '2.0', second.sandbox_id)
        rows.append(dict(case='system_config_write_does_not_change_second'))
        await run(first, f'dpkg --purge {PACKAGE}')
        assert not await call(first.files.exists, CONFIG)
        assert not await call(first.files.exists, EXECUTABLE)
        await check(second, '2.0', second.sandbox_id)
        rows.append(dict(case='package_removal_does_not_change_second'))
        if args.task_profile:
            rows.append(dict(case='failed_initializer_retains_partial_state',
                             result=await failed_initializer(first)))
            assert not await call(second.files.exists, FAILED_INIT)
        assert await call(first.kill)
        live.remove(first)
        await check(second, '2.0', second.sandbox_id)
        rows.append(dict(case='destroying_first_preserves_second'))
        fresh = await call(cls.create, template='base', timeout=300,
                           allow_internet_access=False, **owner)
        live.append(fresh)
        await clean(fresh)
        await check(second, '2.0', second.sandbox_id)
        rows.append(dict(case='replacement_starts_without_package_or_system_writes'))
        return dict(sdk=cls.__name__, cases=rows, vm_count=3)
    finally:
        for sandbox in reversed(live):
            assert await call(sandbox.kill), 'destruction not confirmed'


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = [await exercise_client(cls, args) for cls in (Sandbox, AsyncSandbox)]
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          expected_resources=dict(cpus=args.expected_cpus,
                                                  memory_mib=args.expected_memory_mib),
                          task_profile=args.task_profile,
                          scope='private system files and local Debian package databases; '
                                'new VM has no prior writes; supervisor must audit host '
                                'resources/base; no external dependencies or Harbor tasks'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    parser.add_argument('--expected-cpus', type=int, default=1)
    parser.add_argument('--expected-memory-mib', type=int, default=512)
    parser.add_argument('--task-profile', action='store_true',
                        help='also check root/ubuntu data calls and failed guest initialization')
    asyncio.run(exercise(parser.parse_args()))
