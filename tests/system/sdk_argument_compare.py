"""Same-input foreground/background user/cwd rejection and context cases."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
from contextlib import AsyncExitStack
from functools import partial
import json
import shlex

from e2b.exceptions import AuthenticationException, InvalidArgumentException
from e2b.sandbox.filesystem.filesystem import FileType
from sdk_command_input import KnownCommand, call, public_pid
from sdk_file_compare import freeze, info
from sdk_observations import Calls, Capture

ENV = {'SP_ARGUMENT_VALUE': 'spaces and 中文\nsecond line', 'SP_ARGUMENT_EMPTY': '',
       'SP_ARGUMENT_PID_TEXT': '4321'}
ARGV = ['', 'spaces and \' " < > &', '中文', '4321']
STDERR = 'argument stderr 中文\n'
PROGRAM = ("import json,os,pwd,sys; print(json.dumps(dict(cwd=os.getcwd(), "
           "user=pwd.getpwuid(os.geteuid()).pw_name, argv=sys.argv[1:], "
           "env={k:os.environ.get(k) for k in " + repr(list(ENV)) + "}), "
           "ensure_ascii=False,sort_keys=True,separators=(',',':'))); "
           "print(" + repr(STDERR.rstrip('\n')) + ",file=sys.stderr)")
CONTEXT = 'exec /usr/bin/python3 -I -c ' + shlex.quote(PROGRAM) + ' ' + shlex.join(ARGV)


def context_stdout(directory, user):
    return json.dumps(dict(cwd=directory, user=user, argv=ARGV, env=ENV),
                      ensure_ascii=False, sort_keys=True, separators=(',', ':')) + '\n'


def rejections(directory, unknown):
    rows = [('unknown_user', unknown, directory + '/public', AuthenticationException),
            ('unknown_user_missing_cwd', unknown, directory + '/missing', AuthenticationException)]
    for user in ('root', 'ubuntu'):
        for name, path in [('missing_cwd', '/missing-\'"<>&中文'), ('dangling_cwd', '/dangling'),
                           ('file_cwd', '/ordinary'), ('file_parent_cwd', '/ordinary/child')]:
            rows.append((name + '-' + user, user, directory + path, InvalidArgumentException))
    for name, path in [('denied_cwd', '/private'), ('denied_link', '/private-link'),
                       ('missing_private_child', '/private/missing')]:
        rows.append((name, 'ubuntu', directory + path, InvalidArgumentException))
    return rows


class RecordedCommand(KnownCommand):
    def __init__(self, calls, original):
        super().__init__(calls.sandbox.commands, original)
        self.calls = calls

    async def kill(self, **kwargs):
        async def signal(pid, **options):
            assert pid == self.pid
            return await super(RecordedCommand, self).kill(**options)
        return await self.calls.operation('commands.kill', signal, self.pid, **kwargs)

    async def disconnect(self, handle):
        if id(handle) in self.closed:
            return
        async def close():
            await super(RecordedCommand, self).disconnect(handle)
        await self.calls.operation('handle.disconnect', close, evidence=dict(handle_pid=handle.pid))


async def argument_run(observed, name, command, *, user, cwd, background, expected=None, physical=None):
    row = dict(method='commands.argument_case', name=name, background=background,
               rejected=expected is not None, status='incomplete', calls=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    kwargs = dict(user=user, cwd=cwd, envs=ENV, background=background, timeout=15, request_timeout=15)
    if not background:
        result = await calls.invoke('commands.run', command, expected=expected, **kwargs)
    else:
        capture = {channel: Capture() for channel in ('stdout', 'stderr')}
        callbacks = {'on_' + channel: item for channel, item in capture.items()}
        target = partial(calls.sandbox.commands.run, **callbacks) if calls.asynchronous else calls.sandbox.commands.run
        async with AsyncExitStack() as stack:
            owned = None
            async def start(text, **options):
                nonlocal owned
                handle = await call(target, text, **options)
                row['pid'] = handle.pid  # Preserve even an invalid returned identity.
                owned = await stack.enter_async_context(RecordedCommand(calls, handle))
                return handle
            handle = await calls.operation('commands.run', start, command, expected=expected,
                serialize=lambda returned: {'pid': returned.pid},
                evidence=dict(callbacks={channel: item.parts for channel, item in capture.items()}), **kwargs)
            if expected is None:
                result = await calls.operation('handle.wait', partial(owned.wait, handle,
                    **({} if calls.asynchronous else callbacks)), evidence=dict(handle_pid=owned.pid))
                assert {channel: ''.join(item.parts) for channel, item in capture.items()} == {
                    'stdout': result.stdout, 'stderr': result.stderr}, 'callback output differs from terminal'
            else:
                result = handle  # Expected RPC exception, not a returned subscription.
                assert all(not item.parts for item in capture.values()), 'rejected Start emitted output'
    if expected is None:
        actual = (result.stdout, result.stderr, result.exit_code, result.error)
        assert actual == (context_stdout(physical or cwd, user), STDERR, 0, None), actual
    row['status'] = 'complete'
    return row


async def argument_cases(observed, inputs):
    directory, unknown = inputs['directory'] + '/commands-context', inputs['unknown_user']
    quoted = shlex.quote
    setup = '! getent passwd ' + quoted(unknown) + ' && mkdir -m 0777 ' + quoted(directory)
    setup += ' && mkdir -m 0777 ' + quoted(directory + '/public')
    setup += ' && mkdir -m 0700 ' + quoted(directory + '/private')
    setup += ' && printf original > ' + quoted(directory + '/ordinary')
    setup += ' && chmod 0644 ' + quoted(directory + '/ordinary')
    for target, link in [('public', 'public-link'), ('private', 'private-link'), ('missing', 'dangling')]:
        setup += ' && ln -s ' + quoted(directory + '/' + target) + ' ' + quoted(directory + '/' + link)
    result = await observed.invoke('commands.run', setup, user='root', timeout=15)
    assert (result.stdout, result.stderr, result.exit_code, result.error) == ('', '', 0, None)
    await freeze(observed, directory, inputs['mtime'])
    for path, kind, mode, size in [('public', FileType.DIR, 0o777, None),
                                  ('private', FileType.DIR, 0o700, None),
                                  ('ordinary', FileType.FILE, 0o644, 8)]:
        await info(observed, directory + '/' + path, inputs['mtime'],
                   owner='root', kind=kind, mode=mode, size=size)
    for background in (False, True):
        for user, path, physical in [('root', 'public', 'public'), ('ubuntu', 'public', 'public'),
                                     ('root', 'private', 'private'), ('root', 'private-link', 'private'),
                                     ('root', 'public-link', 'public'), ('ubuntu', 'public-link', 'public')]:
            await argument_run(observed, 'valid-' + user + '-' + path, CONTEXT,
                user=user, cwd=directory + '/' + path, physical=directory + '/' + physical, background=background)
        for index, (name, user, cwd, expected) in enumerate(rejections(directory, unknown)):
            marker = directory + '/rejected-' + str(int(background)) + '-' + str(index)
            source = 'with open(' + repr(marker) + ", 'xb') as f: f.write(b'unexpected\\n')"
            command = 'exec /usr/bin/python3 -I -c ' + quoted(source)
            await argument_run(observed, name, command, user=user, cwd=cwd,
                                background=background, expected=expected)
            exists = await observed.invoke('files.exists', marker, user='root', request_timeout=15)
            assert exists is False, 'rejected Start left its unique marker'
            await argument_run(observed, 'healthy-after-' + name, CONTEXT, user='ubuntu',
                                cwd=directory + '/public', background=False)
    original = await observed.invoke('files.read', directory + '/ordinary', user='root', request_timeout=15)
    assert original == 'original'


def comparable_argument(original):
    row = copy.deepcopy(original)
    assert row['status'] == 'complete'
    assert type(row['background']) is bool and type(row['rejected']) is bool
    calls = row['calls']
    assert calls and all(item['status'] == 'complete' for item in calls)
    assert calls[0]['method'] == 'commands.run'
    assert calls[0]['kwargs']['background'] is row['background']
    if row['background'] and not row['rejected']:
        pid = public_pid(row['pid'])
        assert [item['method'] for item in calls] == ['commands.run', 'handle.wait', 'handle.disconnect']
        assert set(calls[0]['result']) == {'pid'}
        assert type(calls[0]['result']['pid']) is int and calls[0]['result']['pid'] == pid
        row['pid'] = calls[0]['result']['pid'] = '<original-command>'
        for item in calls[1:]:
            assert type(item['handle_pid']) is int and item['handle_pid'] == pid
            item['handle_pid'] = '<original-command>'
    else:
        assert len(calls) == 1 and 'pid' not in row
        assert ('error' in calls[0]) is row['rejected']
    for item in calls:
        if 'callbacks' in item:
            assert set(item['callbacks']) == {'stdout', 'stderr'}
            item['callbacks'] = {key: ''.join(parts) for key, parts in item['callbacks'].items()}
    return row
