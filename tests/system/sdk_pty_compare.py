"""Original SDK PTY observations in the shared Go/Rust input plan."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import asyncio
import base64
import copy
from functools import partial
import json
import shlex
import struct

from e2b.sandbox.commands.command_handle import CommandExitException, PtySize
from sdk_command_input import call
from sdk_background_compare import valid_pid
from sdk_observations import Calls, value
from sdk_pty import (GUEST_PROGRAM, HEADER, READY_HEADER, OUTPUT_LIMIT, PAYLOAD, VALUE,
                     bootstrap, configuration)


class Capture:
    def __init__(self, expect_ready=False):
        self.data, self.chunks = bytearray(), []
        self.expect_ready, self.ready_seen = expect_ready, False
        self.ready = asyncio.Event()
        self.loop = asyncio.get_running_loop()

    def __call__(self, chunk):
        assert type(chunk) is bytes and chunk and len(self.data) + len(chunk) <= OUTPUT_LIMIT, 'PTY observation budget'
        self.data.extend(chunk)
        self.chunks.append(value(chunk))
        if self.expect_ready and not self.ready_seen and READY_HEADER in self.data:
            assert self.data.endswith(READY_HEADER), 'output followed ready before test input'
            self.ready_seen = True
            self.loop.call_soon_threadsafe(self.ready.set)


def read_ready(handle, capture):
    # The original sync SDK supports incremental iteration. Stop at the explicit
    # byte marker; wait() later continues the same underlying event stream.
    for stdout, stderr, data in handle:
        assert stdout is None and stderr is None, 'PTY data used a separate text channel'
        capture(data)
        if capture.ready_seen:
            return
    raise AssertionError('PTY stream ended before ready')


def ready_text(ready):
    return json.dumps(ready) + '\n'


def frame(data, ready, *, startup):
    prefix, separator, rest = data.partition(HEADER)
    assert separator and len(rest) >= 4, 'missing PTY data frame'
    assert (prefix.endswith(READY_HEADER) if startup else not prefix), 'unexpected PTY prefix'
    body = json.dumps(dict(ready=ready, resized=[30, 100])).encode()
    # Require the exact canonical bytes produced by the uploaded program; JSON
    # whitespace, duplicate keys and alternative encodings are not normalized.
    assert rest == struct.pack('!I', len(body)) + body + PAYLOAD, 'PTY metadata or binary bytes differ'
    mapped = dict(ready, pid='<original-pty>')
    body = json.dumps(dict(ready=mapped, resized=[30, 100])).encode()
    return prefix + HEADER + struct.pack('!I', len(body)) + body + PAYLOAD


async def pty_case(observed, user, uid, parent, mode):
    assert mode in ('direct', 'reconnect-exit', 'kill')
    directory = parent + '/pty-' + mode
    row = dict(method='pty.case', args=[user, directory, mode], uid=uid, status='incomplete',
               calls=[], subscriptions=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    pty = observed.sandbox.pty
    handles, disconnected = [], set()
    pid, finished, kill_attempted, failure = None, False, False, None
    wide = 65536 if mode == 'reconnect-exit' else 0
    expected = (-1, 'signal: killed') if mode == 'kill' else (
        (17, 'exit status 17') if wide else (0, None))

    async def subscribe(name, target, *args, **kwargs):
        capture = Capture(expect_ready=name == 'start')
        evidence = dict(name=name, chunks=capture.chunks)
        row['subscriptions'].append(evidence)
        bound = partial(target, on_data=capture) if observed.asynchronous else target
        handle = await calls.operation('pty.create' if name == 'start' else 'pty.connect',
            bound, *args, serialize=lambda result: {'pid': result.pid}, **kwargs)
        handles.append((name, handle))
        evidence['pid'] = handle.pid
        valid_pid(handle.pid)
        return handle, capture

    async def disconnect(name, handle):
        disconnected.add(name)
        await calls.operation('handle.disconnect', handle.disconnect,
            evidence=dict(subscription=name, handle_pid=handle.pid))

    async def wait(name, handle, capture):
        target = handle.wait if observed.asynchronous else partial(handle.wait, on_pty=capture)
        async def observe_terminal():
            nonlocal finished
            try:
                result = await call(target)
            except CommandExitException:
                finished = True
                raise
            finished = True
            return result
        result = await calls.operation('handle.wait', observe_terminal,
            expected=CommandExitException if expected[0] else None,
            evidence=dict(subscription=name, handle_pid=handle.pid))
        return result

    def terminal(result):
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (*expected, '', '')

    try:
        setup = await calls.invoke('commands.run', 'mkdir -m 0700 ' + shlex.quote(directory)
            + ' && mkfifo -m 0600 ' + shlex.quote(directory + '/ready'), user=user, timeout=20)
        assert (setup.exit_code, setup.stdout, setup.stderr) == (0, '', '')
        await calls.invoke('files.write', directory + '/program.py', GUEST_PROGRAM, user=user, request_timeout=15)
        handle, capture = await subscribe('start', pty.create,
            size=PtySize(rows=wide + 24, cols=wide + 80), user=user, cwd=directory,
            envs={'SANDPROOF_PTY_VALUE': VALUE}, timeout=60, request_timeout=15)
        pid = row['pid'] = handle.pid
        config = dict(configuration(directory, 17 if wide else 0), report_ready=True)
        await calls.operation('pty.send_stdin', pty.send_stdin, pid, bootstrap(config), request_timeout=15)
        if observed.asynchronous:
            await asyncio.wait_for(capture.ready.wait(), 20)
        else:
            await call(read_ready, handle, capture)
        ready = dict(pid=pid, uid=uid, cwd=directory, value=VALUE, size=[24, 80], tty=[True]*3)
        row['ready'] = ready
        result = await calls.operation('commands.run', observed.sandbox.commands.run,
            'cat ' + shlex.quote(directory + '/ready'), user=user, timeout=20,
            evidence=dict(pty_ready=True))
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (0, None, ready_text(ready), '')
        name = 'start'
        if mode == 'reconnect-exit':
            await disconnect('start', handle)
            handle, capture = await subscribe('resumed', pty.connect, pid, timeout=60, request_timeout=15)
            assert handle.pid == pid, 'PTY reconnect changed PID'
            name = 'resumed'
        if mode == 'kill':
            kill_attempted = True
            killed = await calls.operation('pty.kill', pty.kill, pid, request_timeout=15)
            assert killed is True
        else:
            await calls.operation('pty.resize', pty.resize, pid,
                                  PtySize(rows=wide + 30, cols=wide + 100), request_timeout=15)
            for chunk in (b'', PAYLOAD[:513], PAYLOAD[513:]):
                await calls.operation('pty.send_stdin', pty.send_stdin, pid, chunk, request_timeout=15)
        result = await wait(name, handle, capture)
        terminal(result)
        if mode == 'kill':
            assert bytes(capture.data).endswith(READY_HEADER) and HEADER not in capture.data
        else:
            frame(bytes(capture.data), ready, startup=mode == 'direct')
        late, late_capture = await subscribe('late', pty.connect, pid, timeout=20, request_timeout=15)
        assert late.pid == pid, 'retained PTY Connect changed PID'
        terminal(await wait('late', late, late_capture))
        assert not late_capture.data, 'retained PTY replayed output'
        marker = await calls.invoke('files.read', directory + '/started', user=user, request_timeout=15)
        assert marker == 'once\n', 'PTY command did not execute exactly once'
    except BaseException as error:
        failure = error
    cleanup = []
    if pid is not None and not finished and not kill_attempted:
        try:
            await calls.operation('pty.kill', pty.kill, pid, request_timeout=15)
        except BaseException as error:
            cleanup.append(error)
    for name, handle in reversed(handles):
        if name not in disconnected:
            try:
                await disconnect(name, handle)
            except BaseException as error:
                cleanup.append(error)
    if cleanup:
        raise BaseExceptionGroup('paired PTY and cleanup failed', ([failure] if failure is not None else []) + cleanup)
    if failure is not None:
        raise failure
    row['status'] = 'complete'
    return row


def comparable_pty(original):
    row = copy.deepcopy(original)
    pid = row['pid']
    valid_pid(pid)
    assert row['status'] == 'complete'
    ready = dict(row['ready'])
    def replace(item, key):
        assert type(item[key]) is int and item[key] == pid, 'reference differs from original PTY PID'
        item[key] = '<original-pty>'
    replace(row, 'pid')
    replace(row['ready'], 'pid')
    starts = ready_reads = 0
    for operation in row['calls']:
        assert operation['status'] == 'complete', 'incomplete PTY operation'
        method = operation['method']
        if method == 'pty.create':
            starts += 1
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method == 'pty.connect':
            replace(operation['args'], 0)
            assert set(operation['result']) == {'pid'}
            replace(operation['result'], 'pid')
        elif method in ('pty.send_stdin', 'pty.resize', 'pty.kill'):
            replace(operation['args'], 0)
        elif method in ('handle.wait', 'handle.disconnect'):
            replace(operation, 'handle_pid')
        else:
            assert method in ('commands.run', 'files.write', 'files.read'), 'unknown PTY operation'
        if operation.get('pty_ready'):
            ready_reads += 1
            assert method == 'commands.run' and operation['result']['stdout'] == ready_text(ready)
            operation['result']['stdout'] = ready_text(row['ready'])
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    assert starts == ready_reads == 1, 'needs one original PTY Start and one readiness observation'
    mode = row['args'][2]
    names = [item['name'] for item in row['subscriptions']]
    assert names == (['start', 'resumed', 'late'] if mode == 'reconnect-exit' else ['start', 'late'])
    for subscription in row['subscriptions']:
        replace(subscription, 'pid')
        data = b''
        for chunk in subscription.pop('chunks'):
            assert set(chunk) == {'base64'}
            data += base64.b64decode(chunk['base64'], validate=True)
            assert len(data) <= OUTPUT_LIMIT
        if subscription['name'] == 'late':
            assert not data, 'retained PTY output'
        elif subscription['name'] == 'start' and mode != 'direct':
            assert data.endswith(READY_HEADER) and HEADER not in data, 'bad startup boundary'
        else:
            data = frame(data, ready, startup=subscription['name'] == 'start')
        subscription['data'] = value(data)
    return row
