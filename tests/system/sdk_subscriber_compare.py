"""Concurrent original command subscriptions in the paired Go/Rust SDK plan."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
from functools import partial
import hashlib
import shlex
from types import SimpleNamespace

from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_background_compare import BINARY, TEXT, command, valid_pid
from sdk_command_input import KnownCommand, call
from sdk_observations import Calls, Capture

MODES = ('detach-start', 'detach-peer')
RESOURCE_PHASES = ('subscribed', 'detached', 'terminal')


class RecordedHandle:
    """Keep the original SDK handle; record each one-shot disconnect attempt."""
    def __init__(self, calls, name, original):
        self.calls, self.name, self.original = calls, name, original
        self.pid = original.pid

    async def wait(self, **kwargs):
        # KnownCommand observes the actual End before the outer recorder can
        # reject its exit category, encoding or output validation.
        return await call(self.original.wait, **kwargs)

    async def disconnect(self):
        await self.calls.operation('handle.disconnect', self.original.disconnect,
            evidence=dict(subscription=self.name, handle_pid=self.pid))


async def subscriber_case(observed, user, parent, mode, *, resource_sample=None):
    assert mode in MODES
    directory = parent + '/subscribers-' + mode
    row = dict(method='commands.subscriber_case', args=[user, directory, mode],
               status='incomplete', calls=[], subscriptions=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    commands = observed.sandbox.commands
    code = 0 if mode == 'detach-start' else 17
    expected = (code, 'exit status 17' if code else None)

    async def sample(phase):
        if resource_sample is not None:
            await resource_sample(f'subscribers/{user}/{mode}/{phase}')

    async def subscribe(name, *args, **kwargs):
        capture = {channel: Capture() for channel in ('stdout', 'stderr')}
        evidence = dict(name=name, callbacks={key: value.parts for key, value in capture.items()})
        row['subscriptions'].append(evidence)
        method = 'commands.run' if name == 'start' else 'commands.connect'
        target = commands.run if name == 'start' else commands.connect
        if observed.asynchronous:
            target = partial(target, **{'on_' + key: value for key, value in capture.items()})
        original = await calls.operation(method, target, *args,
            serialize=lambda handle: {'pid': handle.pid}, **kwargs)
        evidence['pid'] = original.pid
        # Ownership validates after retaining the handle, including bad replies.
        return RecordedHandle(calls, name, original), capture

    async def kill(pid, **kwargs):
        return await calls.operation('commands.kill', commands.kill, pid, **kwargs)

    setup = await calls.invoke('commands.run', 'mkdir -m 0700 ' + shlex.quote(directory)
        + ' && mkfifo -m 0600 ' + shlex.quote(directory + '/ready'), user=user, timeout=15)
    assert (setup.exit_code, setup.error, setup.stdout, setup.stderr) == (0, None, '', '')
    start, capture = await subscribe('start', command(directory, code), user=user, cwd=parent,
                                    stdin=True, background=True, timeout=60, request_timeout=15)
    async with KnownCommand(SimpleNamespace(kill=kill), start) as owned:
        row['pid'] = owned.pid
        subscriptions = [('start', start, capture)]
        for name in ('peer-a', 'peer-b'):
            handle, capture = await subscribe(name, owned.pid, timeout=60, request_timeout=15)
            owned.add(handle)
            subscriptions.append((name, handle, capture))
        # All three original Start responses have been received before release.
        # The private program emits no output until stdin EOF; FIFO readiness
        # confirms execution without racing sleeps or unacknowledged old data.
        ready = await calls.invoke('commands.run', 'cat ' + shlex.quote(directory + '/ready'),
                                   user=user, timeout=15)
        assert (ready.exit_code, ready.error, ready.stdout, ready.stderr) == (0, None, 'ready\n', '')
        await sample('subscribed')
        detached = row['detached'] = 'start' if mode == 'detach-start' else 'peer-a'
        for name, handle, _ in subscriptions:
            if name == detached:
                await owned.disconnect(handle)
        await sample('detached')
        await calls.operation('commands.send_stdin', commands.send_stdin, owned.pid, TEXT, request_timeout=15)
        await calls.operation('commands.send_stdin', commands.send_stdin, owned.pid, BINARY, request_timeout=15)
        await calls.operation('commands.close_stdin', commands.close_stdin, owned.pid, request_timeout=15)
        payload = TEXT.encode() + BINARY
        stdout = f'{len(payload)} {hashlib.sha256(payload).hexdigest()}\n'
        stderr = 'paired stderr 中文\n'

        async def wait(name, handle, capture, out, err):
            options = {} if observed.asynchronous else {'on_' + key: value for key, value in capture.items()}
            result = await calls.operation('handle.wait', partial(owned.wait, handle, **options),
                expected=CommandExitException if code else None,
                evidence=dict(subscription=name, handle_pid=handle.pid))
            assert (result.exit_code, result.error, result.stdout, result.stderr) == (*expected, out, err)
            assert {key: ''.join(value.parts) for key, value in capture.items()} == {'stdout': out, 'stderr': err}

        for name, handle, capture in subscriptions:
            if name == detached:
                assert all(not value.parts for value in capture.values()), 'detached subscriber received output'
            else:
                await wait(name, handle, capture, stdout, stderr)
        late, capture = await subscribe('late', owned.pid, timeout=20, request_timeout=15)
        owned.add(late)
        await wait('late', late, capture, '', '')
        marker = await calls.invoke('files.read', directory + '/started', user=user, request_timeout=15)
        assert marker == 'once\n', 'subscribing executed the command again'
        await sample('terminal')
    row['status'] = 'complete'
    return row


def comparable_subscribers(original):
    """Only explicit validated PID references and callback segmentation may vary."""
    row = copy.deepcopy(original)
    pid = row['pid']
    valid_pid(pid)
    assert row['status'] == 'complete'
    def replace(item, key):
        assert type(item[key]) is int and item[key] == pid, 'reference differs from original PID'
        item[key] = '<original-command>'
    replace(row, 'pid')
    starts = connects = 0
    for operation in row['calls']:
        assert operation['status'] == 'complete'
        method = operation['method']
        if method == 'commands.run' and operation['kwargs'].get('background') is True:
            starts += 1
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method == 'commands.connect':
            connects += 1
            replace(operation['args'], 0)
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method in ('commands.send_stdin', 'commands.close_stdin', 'commands.kill'):
            replace(operation['args'], 0)
        elif method in ('handle.wait', 'handle.disconnect'):
            replace(operation, 'handle_pid')
        else:
            assert method in ('commands.run', 'files.read'), 'unknown subscriber operation'
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    assert (starts, connects) == (1, 3)
    assert [item['name'] for item in row['subscriptions']] == ['start', 'peer-a', 'peer-b', 'late']
    for subscription in row['subscriptions']:
        replace(subscription, 'pid')
        subscription['callbacks'] = {key: ''.join(parts) for key, parts in subscription['callbacks'].items()}
    return row
