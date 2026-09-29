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

from sdk_command_input import call
from sdk_server_smoke import options


PACKAGE = 'sandproof-private-root-probe'
CONFIG = '/etc/sandproof-private-root-probe.conf'
EXECUTABLE = '/usr/local/bin/sandproof-private-root-probe'
STAGING = '/tmp/sandproof-private-root-package'


async def run(sandbox, command):
    result = await call(sandbox.commands.run, command, timeout=30)
    assert result.exit_code == 0
    return result.stdout


async def clean(sandbox):
    for path in (CONFIG, EXECUTABLE, STAGING):
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
    await call(sandbox.files.write, STAGING + '/DEBIAN/control', control)
    await call(sandbox.files.write, STAGING + CONFIG, identity + '\n')
    await call(sandbox.files.write, STAGING + EXECUTABLE,
               '#!/bin/sh\nprintf "%s\\n" ' + shlex.quote(identity) + '\n')
    await run(sandbox, f'chmod 0755 {STAGING + EXECUTABLE} && '
                      f'dpkg-deb --build --root-owner-group {STAGING} {STAGING}.deb && '
                      f'dpkg --install {STAGING}.deb')


async def installed(sandbox, version, identity, config=None):
    assert await call(sandbox.files.read, CONFIG) == (config or identity) + '\n'
    assert await run(sandbox, EXECUTABLE) == identity + '\n'
    assert await run(sandbox, f"dpkg-query -W -f='${{Version}} ${{Status}}' {PACKAGE}") == (
        version + ' install ok installed')
    info = await call(sandbox.get_info)
    assert (info.cpu_count, info.memory_mb) == (1, 512)


async def exercise_client(cls, args):
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    live = []
    rows = []
    try:
        for _ in range(2):
            live.append(await call(cls.create, template='base', timeout=300,
                                   allow_internet_access=False, **owner))
        first, second = live
        await clean(first)
        await clean(second)
        await install(first, '1.0', first.sandbox_id)
        await installed(first, '1.0', first.sandbox_id)
        await clean(second)
        rows.append(dict(case='first_install_does_not_change_second'))
        await install(second, '2.0', second.sandbox_id)
        await installed(second, '2.0', second.sandbox_id)
        await installed(first, '1.0', first.sandbox_id)
        rows.append(dict(case='same_paths_and_package_have_independent_contents'))
        changed = first.sandbox_id + '-modified'
        await call(first.files.write, CONFIG, changed + '\n')
        await installed(first, '1.0', first.sandbox_id, config=changed)
        await installed(second, '2.0', second.sandbox_id)
        rows.append(dict(case='system_config_write_does_not_change_second'))
        await run(first, f'dpkg --purge {PACKAGE}')
        assert not await call(first.files.exists, CONFIG)
        assert not await call(first.files.exists, EXECUTABLE)
        await installed(second, '2.0', second.sandbox_id)
        rows.append(dict(case='package_removal_does_not_change_second'))
        assert await call(first.kill)
        live.remove(first)
        await installed(second, '2.0', second.sandbox_id)
        rows.append(dict(case='destroying_first_preserves_second'))
        fresh = await call(cls.create, template='base', timeout=300,
                           allow_internet_access=False, **owner)
        live.append(fresh)
        await clean(fresh)
        await installed(second, '2.0', second.sandbox_id)
        rows.append(dict(case='replacement_starts_without_package_or_system_writes'))
        return dict(sdk=cls.__name__, cases=rows, vm_count=3)
    finally:
        for sandbox in reversed(live):
            assert await call(sandbox.kill), 'destruction not confirmed'


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    results = [await exercise_client(cls, args) for cls in (Sandbox, AsyncSandbox)]
    print(json.dumps(dict(sdk_version='2.51.0', results=results,
                          scope='private system files and local Debian package databases; '
                                'new VM has no prior writes; supervisor must audit host '
                                'resources/base; no external dependencies or Harbor tasks'), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    asyncio.run(exercise(parser.parse_args()))
