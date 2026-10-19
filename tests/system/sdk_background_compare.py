"""Same-input background command slice for the paired original SDK probe."""
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

from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_command_input import READ_INPUT, call
from sdk_observations import Calls, Capture

TEXT = 'first line and 中文\n'
BINARY = bytes(range(256)) * 257
MODES = ('direct', 'reconnect-exit', 'kill')


def command(directory, code):
    # The exclusive marker detects execution again, and the FIFO puts kill and
    # reconnect after the real program has started. No output precedes stdin EOF.
    source = (
        'import sys\n'
        + 'with open(' + repr(directory + '/started') + ", 'xb') as f: f.write(b'once\\n')\n"
        + 'with open(' + repr(directory + '/ready') + ", 'wb', buffering=0) as f: f.write(b'ready\\n')\n"
        + READ_INPUT + "\nprint('paired stderr 中文', file=sys.stderr)\nsys.exit(" + str(code) + ')'
    )
    return 'exec /usr/bin/python3 -I -c ' + shlex.quote(source)


def valid_pid(pid):
    assert type(pid) is int and 0 < pid <= 4194304, 'invalid public PID'


async def background_case(observed, user, parent, mode):
    assert mode in MODES
    directory = parent + '/background-' + mode
    row = dict(method='background.case', args=[user, directory, mode], status='incomplete',
               calls=[], subscriptions=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    commands = observed.sandbox.commands
    handles, disconnected = [], set()
    pid, finished, kill_attempted, failure = None, False, False, None
    expected = (-1, 'signal: killed') if mode == 'kill' else (
        (17, 'exit status 17') if mode == 'reconnect-exit' else (0, None))

    async def subscription(name, target, *args, **kwargs):
        capture = {channel: Capture() for channel in ('stdout', 'stderr')}
        evidence = dict(name=name, callbacks={channel: item.parts for channel, item in capture.items()})
        row['subscriptions'].append(evidence)
        callback_kwargs = {'on_' + channel: item for channel, item in capture.items()}
        bound = partial(target, **callback_kwargs) if observed.asynchronous else target
        handle = await calls.operation('commands.run' if name == 'start' else 'commands.connect',
            bound, *args, serialize=lambda result: {'pid': result.pid}, **kwargs)
        handles.append((name, handle))  # Close even a returned, wrong-PID subscription.
        evidence['pid'] = handle.pid
        valid_pid(handle.pid)
        return handle, capture

    async def handle_call(name, handle, method, *args, **kwargs):
        return await calls.operation('handle.' + method, getattr(handle, method), *args,
            evidence=dict(subscription=name, handle_pid=handle.pid), **kwargs)

    async def disconnect(name, handle):
        disconnected.add(name)  # An unknown close is never retried.
        await handle_call(name, handle, 'disconnect')

    async def wait(name, handle, capture):
        bound = handle.wait if observed.asynchronous else partial(handle.wait,
            on_stdout=capture['stdout'], on_stderr=capture['stderr'])
        async def observe_terminal():
            nonlocal finished
            # Receipt of End is independent of the expected exit category.
            # A mismatch must fail validation without another process signal.
            try:
                result = await call(bound)
            except CommandExitException:
                finished = True
                raise
            finished = True
            return result
        result = await calls.operation('handle.wait', observe_terminal,
            expected=CommandExitException if expected[0] else None,
            evidence=dict(subscription=name, handle_pid=handle.pid))
        return result

    def terminal(result, capture, stdout, stderr):
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (*expected, stdout, stderr)
        assert {channel: ''.join(item.parts) for channel, item in capture.items()} == {
            'stdout': stdout, 'stderr': stderr}, 'callback output differs from terminal result'

    try:
        setup = await calls.invoke('commands.run', 'mkdir -m 0700 ' + shlex.quote(directory)
            + ' && mkfifo -m 0600 ' + shlex.quote(directory + '/ready'), user=user, timeout=15)
        assert (setup.exit_code, setup.stdout, setup.stderr) == (0, '', '')
        handle, capture = await subscription('start', commands.run,
            command(directory, max(expected[0], 0)), user=user, cwd=parent, stdin=True,
            background=True, timeout=60, request_timeout=15)
        pid = row['pid'] = handle.pid
        ready = await calls.invoke('commands.run', 'cat ' + shlex.quote(directory + '/ready'),
                                    user=user, timeout=15)
        assert (ready.exit_code, ready.stdout, ready.stderr) == (0, 'ready\n', '')
        active_name, payload = 'start', b''
        if mode == 'reconnect-exit':
            await handle_call('start', handle, 'send_stdin', TEXT, request_timeout=15)
            await disconnect('start', handle)
            assert all(not item.parts for item in capture.values()), 'output preceded stdin EOF'
            handle, capture = await subscription('resumed', commands.connect, pid, timeout=60, request_timeout=15)
            assert handle.pid == pid, 'reconnect changed PID'
            active_name, payload = 'resumed', TEXT.encode() + BINARY
            await calls.operation('commands.send_stdin', commands.send_stdin, pid, BINARY, request_timeout=15)
        await handle_call(active_name, handle, 'send_stdin', b'', request_timeout=15)
        if mode == 'kill':
            kill_attempted = True
            killed = await calls.operation('commands.kill', commands.kill, pid, request_timeout=15)
            assert killed is True
        elif mode == 'reconnect-exit':
            await calls.operation('commands.close_stdin', commands.close_stdin, pid, request_timeout=15)
        else:
            await handle_call(active_name, handle, 'close_stdin', request_timeout=15)
        stdout = '' if mode == 'kill' else str(len(payload)) + ' ' + hashlib.sha256(payload).hexdigest() + '\n'
        stderr = '' if mode == 'kill' else 'paired stderr 中文\n'
        result = await wait(active_name, handle, capture)
        terminal(result, capture, stdout, stderr)
        late, late_capture = await subscription('late', commands.connect, pid, timeout=20, request_timeout=15)
        assert late.pid == pid, 'retained Connect changed PID'
        terminal(await wait('late', late, late_capture), late_capture, '', '')
        marker = await calls.invoke('files.read', directory + '/started', user=user, request_timeout=15)
        assert marker == 'once\n', 'command did not execute exactly once'
    except BaseException as error:
        failure = error
    cleanup = []
    if pid is not None and not finished and not kill_attempted:
        try:
            await calls.operation('commands.kill', commands.kill, pid, request_timeout=15)
        except BaseException as error:
            cleanup.append(error)
    for name, handle in reversed(handles):
        if name not in disconnected:
            try:
                await disconnect(name, handle)
            except BaseException as error:
                cleanup.append(error)
    if cleanup:
        raise BaseExceptionGroup('background case and cleanup failed',
                                 ([failure] if failure is not None else []) + cleanup)
    if failure is not None:
        raise failure
    row['status'] = 'complete'
    return row


def comparable_background(original):
    """Replace only explicit references to this case's one validated Start PID.

    Raw rows remain intact. Never rewrite output, error text, command arguments,
    file data, arbitrary integers or metadata by searching for the PID value.
    """
    row = copy.deepcopy(original)
    pid = row['pid']
    valid_pid(pid)
    assert row['status'] == 'complete'
    def replace(item, key):
        assert type(item[key]) is int and item[key] == pid, 'reference differs from original PID'
        item[key] = '<original-command>'
    replace(row, 'pid')
    starts = 0
    for operation in row['calls']:
        assert operation['status'] == 'complete', 'incomplete background operation'
        method = operation['method']
        if method == 'commands.run' and operation['kwargs'].get('background') is True:
            starts += 1
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method == 'commands.connect':
            replace(operation['args'], 0)
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method in ('commands.send_stdin', 'commands.close_stdin', 'commands.kill'):
            replace(operation['args'], 0)
        elif method in ('handle.wait', 'handle.send_stdin', 'handle.close_stdin', 'handle.disconnect'):
            replace(operation, 'handle_pid')
        else:
            assert method in ('commands.run', 'files.read'), 'unknown background operation'
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    assert starts == 1, 'case needs exactly one original Start'
    names = [item['name'] for item in row['subscriptions']]
    assert names == (['start', 'resumed', 'late'] if row['args'][2] == 'reconnect-exit' else ['start', 'late'])
    for subscription in row['subscriptions']:
        replace(subscription, 'pid')
        subscription['callbacks'] = {key: ''.join(parts) for key, parts in subscription['callbacks'].items()}
    return row
