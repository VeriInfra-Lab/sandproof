"""Original SDK checks for per-template/per-instance execution defaults.

Fixed envd resolves nonempty relative file paths against the selected user's
home; command cwd defaults instead use the template WORKDIR. A file user is
not a guest DAC boundary. The host supervisor separately checks VM identity.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import json
import shlex

from sdk_command_input import call


START_COUNT = '/tmp/sandproof-template-start-count'
START_CONTEXT = '/tmp/sandproof-template-start-context'
START_PID = '/tmp/sandproof-template-start-pid'
LITERAL_ARG = '$TEMPLATE_MARKER; touch /tmp/sandproof-template-injected'
START_SCRIPT = (
    f"umask 077; printf 'once\\n' >> {START_COUNT}; "
    "printf '%s\\0' \"$(id -un)\" \"$HOME\" \"$PWD\" \"$TEMPLATE_MARKER\" "
    f'"$LAYERED" "$INSTANCE_MARKER" "$@" > {START_CONTEXT}; '
    f'printf "%s\\n" "$$" > {START_PID}; exec sleep 900'
)
CONTEXT_COMMAND = (
    "printf '%s\\0' \"$(id -un)\" \"$HOME\" \"$PWD\" \"$TEMPLATE_MARKER\" "
    '"$LAYERED" "$INSTANCE_MARKER" "${ONLY_REQUEST-unset}"'
)


def defaults(index):
    return {'env_vars': {'TEMPLATE_MARKER': f'template-{index}', 'LAYERED': f'image-{index}'},
            'user': 'ubuntu' if index == 0 else 'root',
            'workdir': '/tmp' if index == 0 else '/',
            'argv': ['/bin/sh', '-c', START_SCRIPT, 'template-entry', 'literal space', LITERAL_ARG]}


def home(user):
    assert user in ('root', 'ubuntu')
    return '/root' if user == 'root' else '/home/ubuntu'


def expected_context(startup, instance):
    return [startup['user'], home(startup['user']), startup['workdir'],
            startup['env_vars']['TEMPLATE_MARKER'], 'create-' + instance, instance]


async def command_context(sandbox, **kwargs):
    result = await call(sandbox.commands.run, CONTEXT_COMMAND, timeout=15, **kwargs)
    assert result.exit_code == 0 and result.stderr == ''
    fields = result.stdout.split('\0')
    assert fields[-1] == ''
    return fields[:-1]


async def check_files(sandbox, startup, instance, sdk):
    user = startup['user']
    other = 'root' if user == 'ubuntu' else 'ubuntu'
    rows = []
    for kind, selected, path in (
        ('relative', user, f'template-default-{sdk}.txt'),
        ('tilde', user, f'~/template-tilde-{sdk}.txt'),
        ('explicit-user', other, f'template-explicit-{sdk}.txt'),
    ):
        kwargs = {'user': other} if kind == 'explicit-user' else {}
        absolute = home(selected) + '/' + path.removeprefix('~/')
        content = json.dumps({'instance': instance, 'template': startup['env_vars']['TEMPLATE_MARKER'],
                              'sdk': sdk, 'case': kind}, sort_keys=True) + '\n'
        written = await call(sandbox.files.write, path, content, **kwargs)
        assert written.path == absolute
        assert await call(sandbox.files.read, path, **kwargs) == content
        assert await call(sandbox.files.read, absolute, user='root') == content
        info = await call(sandbox.files.get_info, path, **kwargs)
        assert info.path == absolute and info.size == len(content.encode())
        owner = await call(sandbox.commands.run, 'stat -c %U -- ' + shlex.quote(absolute),
                           user='root', timeout=15)
        assert owner.stdout.strip() == selected
        rows.append({'path': absolute, 'content': content, 'owner': selected})
    return rows


async def exercise(sandbox, startup, instance):
    sdk = type(sandbox).__name__
    expected = expected_context(startup, instance)
    assert await command_context(sandbox) == expected + ['unset']
    other = 'root' if startup['user'] == 'ubuntu' else 'ubuntu'
    # Both directories are part of the approved boot environment. Minimal
    # Ubuntu images need not contain /var/tmp, and a missing cwd must fail.
    request_cwd = '/' if startup['workdir'] == '/tmp' else '/tmp'
    actual = await command_context(sandbox, user=other, cwd=request_cwd,
                                   envs={'LAYERED': 'request-value', 'ONLY_REQUEST': 'one-call'})
    assert actual == [other, home(other), request_cwd, expected[3], 'request-value', instance, 'one-call']
    # Per-request overrides must not mutate envd defaults for the next call.
    assert await command_context(sandbox) == expected + ['unset']
    files = await check_files(sandbox, startup, instance, sdk)
    return {'sdk': sdk, 'default_context': expected, 'request_overrides_are_local': True,
            'files': files}
