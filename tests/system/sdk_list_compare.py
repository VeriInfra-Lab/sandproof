"""Complete original SDK List observations while known commands are held alive."""
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
import time

from e2b.sandbox.commands.command_handle import CommandExitException, PtySize
from sdk_background_compare import valid_pid
from sdk_command_input import call
from sdk_observations import Calls, value

READY = b'\x00sandproof-list-ready\x00'
LIMIT = 128 * 1024
SECONDS, ROUNDS = 15, 128
NAMES = ('root', 'ubuntu', 'pty')
INFO_KEYS = {'pid', 'tag', 'cmd', 'args', 'envs', 'cwd'}


def program(directory, name, code):
    source = ('import os,sys,tty\n'
        "with open(sys.argv[1], 'xb') as f: f.write(b'once\\n')\n"
        'if os.isatty(0): tty.setraw(0)\n'
        'os.write(1, ' + repr(READY) + ')\n'
        'sys.exit(' + str(code) + " if os.read(0, 1) == b'X' else 42)\n")
    return 'exec /usr/bin/python3 -I -c ' + shlex.quote(source) + ' ' + shlex.quote(directory + '/started-' + name)


def specs(directory):
    result = {}
    for name, code in (('root', 0), ('ubuntu', 17), ('pty', -1)):
        pty = name == 'pty'
        envs = {} if name == 'root' else {'SANDPROOF_LIST_VALUE': 'literal 4321 中文', 'EMPTY': ''}
        cwd = None if name == 'root' else directory
        command = program(directory, name, max(code, 0))
        metadata = dict(tag=None, cmd='/bin/bash', args=['-i', '-l'] if pty else ['-l', '-c', command],
                        envs=dict(envs), cwd=cwd)
        if pty:
            metadata['envs'].update(TERM='xterm-256color', LANG='C.UTF-8', LC_ALL='C.UTF-8')
        result[name] = dict(user='root' if name == 'root' else 'ubuntu', pty=pty, code=code,
                            command=command, cwd=cwd, envs=envs, metadata=metadata)
    return result


def shape(entries):
    assert type(entries) is list and len(entries) <= 16, 'List observation count budget'
    assert len(json.dumps(entries).encode()) <= LIMIT, 'List observation byte budget'
    seen = set()
    for item in entries:
        assert set(item) == INFO_KEYS, 'unexpected List fields'
        valid_pid(item['pid'])
        assert item['pid'] not in seen, 'duplicate listed PID'
        seen.add(item['pid'])
        assert item['tag'] is None or type(item['tag']) is str
        assert item['cwd'] is None or type(item['cwd']) is str
        assert type(item['cmd']) is str and type(item['args']) is list and type(item['envs']) is dict
        assert all(type(arg) is str for arg in item['args'])
        assert all(type(key) is type(part) is str for key, part in item['envs'].items())
    return seen


def canonical(entries, names, pids, definitions, retiring=None):
    seen = shape(entries)
    expected = {pids[name] for name in names}
    optional = {pids[retiring]} if retiring is not None else set()
    assert expected <= seen <= expected | optional, 'missing or extra listed process'
    by_pid = {pid: name for name, pid in pids.items()}
    result = []
    for item in entries:
        name = by_pid[item['pid']]
        wanted = dict(pid=item['pid'], **definitions[name]['metadata'])
        assert json.dumps(item, sort_keys=True) == json.dumps(wanted, sort_keys=True), 'List metadata differs'
        result.append(dict(item, pid='<list-' + name + '>'))
    return sorted(result, key=lambda item: item['pid'])


async def observe_removal(calls, evidence, names, pids, definitions, *, retiring=None,
                          baseline=False, clock=time.monotonic, pause=asyncio.sleep):
    evidence.update(names=list(names), retiring=retiring, baseline=baseline, status='incomplete', calls=[])
    observed = Calls(calls.sandbox, calls.asynchronous)
    observed.rows = evidence['calls']
    deadline = clock() + SECONDS
    def remaining():
        left = deadline - clock()
        assert left > 0, 'List observation deadline'
        return left
    for _ in range(ROUNDS):
        listed = await observed.invoke('commands.list', request_timeout=remaining())
        remaining()
        raw = value(listed)
        seen = shape(raw)
        if baseline:
            # Setup/identity commands have already returned End. Wait for an
            # empty service table before this controlled List case begins;
            # never signal a pre-existing or otherwise unknown PID.
            done = not seen
        else:
            canonical(raw, names, pids, definitions, retiring)
            done = pids[retiring] not in seen
        if done:
            evidence['status'] = 'complete'
            return
        await pause(min(0.02, remaining()))
    raise AssertionError('List observation count exceeded')


class Capture:
    def __init__(self, pty, ready):
        self.pty, self.expect_ready = pty, ready
        self.parts = {name: [] for name in ('stdout', 'stderr', 'pty')}
        self.size, self.ready_seen = 0, False
        self.failure = None
        self.event, self.loop = asyncio.Event(), asyncio.get_running_loop()

    def append(self, channel, data):
        try:
            self._append(channel, data)
        except BaseException as error:
            self.failure = error
            self.loop.call_soon_threadsafe(self.event.set)
            raise

    def _append(self, channel, data):
        assert type(data) is (bytes if channel == 'pty' else str)
        self.size += len(data) if channel == 'pty' else len(data.encode())
        assert self.size <= LIMIT, 'List command output budget'
        self.parts[channel].append(value(data))
        if self.expect_ready and not self.ready_seen:
            raw = self.data()
            if not self.pty:
                assert READY.startswith(raw), 'unexpected ready bytes'
            if READY in raw:
                assert raw.endswith(READY) and raw.count(READY) == 1, 'invalid ready boundary'
                self.ready_seen = True
                self.loop.call_soon_threadsafe(self.event.set)

    def data(self):
        assert not self.parts['stderr'], 'unexpected stderr'
        if self.pty:
            assert not self.parts['stdout'], 'PTY used stdout channel'
            return b''.join(base64.b64decode(chunk['base64'], validate=True) for chunk in self.parts['pty'])
        assert not self.parts['pty'], 'command used PTY channel'
        return ''.join(self.parts['stdout']).encode()

    def callbacks(self):
        channels = ('pty',) if self.pty else ('stdout', 'stderr')
        return {'on_' + channel: partial(self.append, channel) for channel in channels}


def read_ready(handle, capture):
    for stdout, stderr, pty in handle:
        for channel, part in (('stdout', stdout), ('stderr', stderr), ('pty', pty)):
            if part is not None:
                capture.append(channel, part)
        if capture.ready_seen:
            return
    raise AssertionError('command ended before List readiness')


async def list_case(observed, inputs):
    directory = inputs['directory'] + '/list'
    row = dict(method='commands.list_case', args=[directory], status='incomplete',
               calls=[], observations=[], subscriptions=[], pids={})
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    definitions = specs(directory)
    pids, states, handles, disconnected = row['pids'], {}, [], set()
    failure = None

    async def subscribe(name, label, start):
        spec = definitions[name]
        capture = Capture(spec['pty'], start)
        evidence = dict(name=name, label=label, channels=capture.parts)
        row['subscriptions'].append(evidence)
        api = observed.sandbox.pty if spec['pty'] else observed.sandbox.commands
        method = ('pty.' if spec['pty'] else 'commands.') + ('create' if spec['pty'] and start else 'run' if start else 'connect')
        target = getattr(api, method.split('.')[1])
        kwargs = dict(timeout=60 if start or label == 'ubuntu-connected' else 20, request_timeout=15)
        args = [] if spec['pty'] and start else [spec['command']] if start else [pids[name]]
        if start:
            kwargs.update(user=spec['user'], cwd=spec['cwd'], envs=spec['envs'])
            if spec['pty']:
                kwargs.update(size=PtySize(rows=24, cols=80))
            else:
                kwargs.update(stdin=True, background=True)
        if observed.asynchronous:
            callbacks = {'on_data': partial(capture.append, 'pty')} if spec['pty'] else capture.callbacks()
            target = partial(target, **callbacks)
        handle = await calls.operation(method, target, *args, serialize=lambda handle: {'pid': handle.pid},
                                       evidence=dict(name=name, subscription=label), **kwargs)
        handles.append((name, label, handle, capture))
        evidence['pid'] = handle.pid
        valid_pid(handle.pid)
        if start:
            assert handle.pid not in pids.values(), 'duplicate original Start PID'
            pids[name] = handle.pid
            states[name] = dict(finished=False, kill_attempted=False)
        else:
            assert handle.pid == pids[name], 'List case Connect changed PID'
        return handle, capture

    async def wait(name, label, handle, capture, original):
        target = handle.wait if observed.asynchronous else partial(handle.wait, **capture.callbacks())
        async def terminal():
            try:
                result = await call(target)
            except CommandExitException:
                states[name]['finished'] = True
                raise
            states[name]['finished'] = True
            return result
        code = definitions[name]['code']
        result = await calls.operation('handle.wait', terminal, expected=CommandExitException if code else None,
            evidence=dict(name=name, subscription=label, handle_pid=handle.pid))
        stdout = READY.decode() if original and not definitions[name]['pty'] else ''
        error = 'signal: killed' if code == -1 else 'exit status 17' if code else None
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (code, error, stdout, '')
        data = capture.data()
        assert (data.endswith(READY) and data.count(READY) == 1
                if original and definitions[name]['pty'] else data == stdout.encode())

    async def snapshot(names):
        listed = await calls.operation('commands.list', observed.sandbox.commands.list,
                                       request_timeout=15, evidence=dict(list_names=list(names)))
        canonical(value(listed), names, pids, definitions)

    async def removal(names, retiring=None, baseline=False):
        evidence = {}
        row['observations'].append(evidence)
        await observe_removal(calls, evidence, names, pids, definitions, retiring=retiring, baseline=baseline)

    async def disconnect(name, label, handle):
        disconnected.add(label)
        await calls.operation('handle.disconnect', handle.disconnect,
            evidence=dict(name=name, subscription=label, handle_pid=handle.pid))

    try:
        result = await calls.invoke('commands.run', 'mkdir -m 0777 ' + shlex.quote(directory), user='root', timeout=15)
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (0, None, '', '')
        await removal([], baseline=True)
        original = {}
        for name in NAMES:
            handle, capture = await subscribe(name, name + '-start', True)
            original[name] = handle, capture
            if definitions[name]['pty']:
                bootstrap = (definitions[name]['command'] + '\n').encode()
                assert len(bootstrap) < 512, 'PTY bootstrap exceeds canonical input boundary'
                await calls.operation('pty.send_stdin', observed.sandbox.pty.send_stdin,
                                      pids[name], bootstrap, request_timeout=15, evidence=dict(name=name))
            if observed.asynchronous:
                await asyncio.wait_for(capture.event.wait(), 20)
            else:
                await call(read_ready, handle, capture)
            if capture.failure is not None:
                raise capture.failure
            assert capture.ready_seen
        await snapshot(NAMES)
        connected = await subscribe('ubuntu', 'ubuntu-connected', False)
        await snapshot(NAMES)
        live = list(NAMES)
        for name in ('pty', 'root', 'ubuntu'):
            if name == 'pty':
                states[name]['kill_attempted'] = True
                killed = await calls.operation('pty.kill', observed.sandbox.pty.kill, pids[name],
                                               request_timeout=15, evidence=dict(name=name))
                assert killed is True
            else:
                await calls.operation('commands.send_stdin', observed.sandbox.commands.send_stdin,
                                      pids[name], b'X', request_timeout=15, evidence=dict(name=name))
            await wait(name, name + '-start', *original[name], True)
            if name == 'ubuntu':
                await wait(name, 'ubuntu-connected', *connected, False)
            live.remove(name)
            await removal(live, retiring=name)
            await snapshot(live)
            late = await subscribe(name, name + '-late', False)
            await wait(name, name + '-late', *late, False)
            await snapshot(live)
        for name in NAMES:
            marker = await calls.invoke('files.read', directory + '/started-' + name, user='root', request_timeout=15)
            assert marker == 'once\n', 'listed process was not executed exactly once'
    except BaseException as error:
        failure = error
    cleanup = []
    for name, state in states.items():
        if not state['finished'] and not state['kill_attempted']:
            state['kill_attempted'] = True
            api = observed.sandbox.pty if definitions[name]['pty'] else observed.sandbox.commands
            try:
                await calls.operation(('pty.' if definitions[name]['pty'] else 'commands.') + 'kill',
                                      api.kill, pids[name], request_timeout=15, evidence=dict(name=name))
            except BaseException as error:
                cleanup.append(error)
    for name, label, handle, _ in reversed(handles):
        if label not in disconnected:
            try:
                await disconnect(name, label, handle)
            except BaseException as error:
                cleanup.append(error)
    if cleanup:
        raise BaseExceptionGroup('List case and cleanup failed', ([failure] if failure is not None else []) + cleanup)
    if failure is not None:
        raise failure
    row['status'] = 'complete'
    return row


def comparable_list(original):
    row = copy.deepcopy(original)
    assert row['status'] == 'complete' and set(row['pids']) == set(NAMES)
    pids, definitions = dict(row['pids']), specs(row['args'][0])
    for pid in pids.values():
        valid_pid(pid)
    assert len(set(pids.values())) == len(NAMES)
    def replace(item, key, name):
        assert type(item[key]) is int and item[key] == pids[name], 'not the original listed PID'
        item[key] = '<list-' + name + '>'
    for name in NAMES:
        replace(row['pids'], name, name)
    starts = []
    for operation in row['calls']:
        assert operation['status'] == 'complete'
        method = operation['method']
        if method == 'commands.list':
            operation['result'] = canonical(operation['result'], operation['list_names'], pids, definitions)
        elif 'name' in operation:
            name = operation['name']
            assert name in NAMES
            if method in ('pty.create', 'commands.run'):
                starts.append(name)
                replace(operation['result'], 'pid', name)
            elif method in ('pty.connect', 'commands.connect'):
                replace(operation['args'], 0, name)
                replace(operation['result'], 'pid', name)
            elif method in ('pty.send_stdin', 'commands.send_stdin', 'pty.kill', 'commands.kill'):
                replace(operation['args'], 0, name)
            else:
                assert method in ('handle.wait', 'handle.disconnect')
                replace(operation, 'handle_pid', name)
        else:
            assert method in ('commands.run', 'files.read')
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    assert starts == list(NAMES)
    for subscription in row['subscriptions']:
        replace(subscription, 'pid', subscription['name'])
        parts = subscription['channels']
        parts['stdout'], parts['stderr'] = ''.join(parts['stdout']), ''.join(parts['stderr'])
        parts['pty'] = value(b''.join(base64.b64decode(c['base64'], validate=True) for c in parts['pty']))
    expected_observations = [([], None, True), (['root', 'ubuntu'], 'pty', False),
                             (['ubuntu'], 'root', False), ([], 'ubuntu', False)]
    assert len(row['observations']) == len(expected_observations)
    for observation, expected in zip(row['observations'], expected_observations):
        assert (observation['names'], observation['retiring'], observation['baseline']) == expected
        assert observation['status'] == 'complete' and 0 < len(observation['calls']) <= ROUNDS
        previous_timeout = SECONDS
        for index, sample in enumerate(observation['calls']):
            assert sample['status'] == 'complete' and sample['method'] == 'commands.list' and sample['args'] == []
            timeout = sample['kwargs']['request_timeout']
            assert type(timeout) in (int, float) and 0 < timeout <= previous_timeout
            assert sample['kwargs'] == dict(request_timeout=timeout) and 'error' not in sample
            previous_timeout = timeout
            seen = shape(sample['result'])
            if observation['baseline']:
                done = not seen
            else:
                canonical(sample['result'], observation['names'], pids, definitions, observation['retiring'])
                done = pids[observation['retiring']] not in seen
            assert done is (index == len(observation['calls']) - 1), 'invalid List observation boundary'
        # Raw sampling stays in the report. Only bounded setup quiescence and
        # exact known-identity removal progress are excluded from equality;
        # stable complete snapshots and all their fields are compared above.
        del observation['calls']
    return row
