"""Same-input, one-live-tag lifecycle in the paired fixed Go/Rust SDK plan.

Duplicate live tags and multiple different live tags are approved differences;
this slice intentionally compares the shared successful and retained lifecycle.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import base64
import asyncio
import copy
from functools import partial
import hashlib
import shlex
from types import SimpleNamespace

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from sdk_background_compare import valid_pid
from sdk_command_input import KnownCommand, READ_INPUT, call
from sdk_list_compare import READY, Capture, canonical, check_observations, observe_removal, read_ready
from sdk_observations import Calls, value
from sdk_tag_rpc import RecordedHandle, TagRpc

GENERATIONS = ('first', 'second')
TAGS = {'root': '', 'ubuntu': 'paired "\\<>&中\n'}
INPUT = bytes(range(256))


def program(directory):
    source = ('import os\n'
        + 'with open(' + repr(directory + '/started') + ", 'xb') as f: f.write(b'once\\n')\n"
        + 'os.write(1, ' + repr(READY) + ')\n' + READ_INPUT
        + "\nprint('paired stderr 中文', file=sys.stderr)\n")
    return 'exec /usr/bin/python3 -I -c ' + shlex.quote(source)


def definitions(parent, tag):
    return {name: {'metadata': dict(tag=tag, cmd='/bin/bash',
        args=['-l', '-c', program(parent + '/' + name)], envs={}, cwd=parent)}
        for name in GENERATIONS}


def not_found(error, selector, wanted):
    assert type(error) is ConnectError
    assert (error.code, error.message) == (Code.NOT_FOUND, f'process with {selector} {wanted} not found')


async def tag_case(observed, user, parent):
    tag, directory = TAGS[user], parent + '/tags'
    row = dict(method='commands.tag_case', args=[user, directory, tag], status='incomplete',
               pids={}, calls=[], observations=[], subscriptions=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    rpc, pids, specs = TagRpc(calls, user), row['pids'], definitions(directory, tag)

    async def removal(retiring=None):
        evidence = {}
        row['observations'].append(evidence)
        await observe_removal(calls, evidence, [], pids, specs,
                              retiring=retiring, baseline=retiring is None)

    async def subscribe(name, label, start=False):
        capture = Capture(False, start)
        entry = dict(generation=name, label=label, callbacks=capture.parts)
        row['subscriptions'].append(entry)
        payload = (dict(tag=tag, stdin=True, process={key: part for key, part in specs[name]['metadata'].items()
                                                    if key != 'tag'})
                   if start else {'process': {'tag': tag}})
        original = await rpc.stream('start' if start else 'connect', payload, capture.callbacks(),
            generation=name, label=label, seconds=20 if label == 'late' else 60)
        entry['pid'] = original.pid
        return RecordedHandle(calls, original, name, label), capture

    async def kill(pid, **kwargs):
        return await calls.operation('commands.kill', observed.sandbox.commands.kill, pid, **kwargs)

    setup = ('mkdir -m 0700 ' + shlex.quote(directory)
             + ''.join(' && mkdir -m 0700 ' + shlex.quote(directory + '/' + name)
                       for name in GENERATIONS))
    result = await calls.invoke('commands.run', setup, user=user, timeout=15)
    assert (result.exit_code, result.error, result.stdout, result.stderr) == (0, None, '', '')
    await removal()
    for name in GENERATIONS:
        original, capture = await subscribe(name, 'start', True)
        async with KnownCommand(SimpleNamespace(kill=kill), original) as owned:
            assert owned.pid not in pids.values(), 'new generation reused an original PID'
            pids[name] = owned.pid
            if observed.asynchronous:
                await asyncio.wait_for(capture.event.wait(), 15)
            else:
                await asyncio.wait_for(call(read_ready, original.original, capture), 15)
            if capture.failure is not None:
                raise capture.failure
            assert capture.ready_seen and capture.data() == READY
            # The marker is consumed by this original stream before reconnect;
            # subsequent output waits for stdin EOF and must not replay it.
            await owned.disconnect(original)
            if name == 'second':
                failure = await rpc.stream('connect', {'process': {'pid': pids['first']}}, {},
                    generation='first', label='retired', seconds=20, expected=ConnectError)
                not_found(failure, 'pid', pids['first'])
            listed = await calls.operation('commands.list', observed.sandbox.commands.list,
                request_timeout=15, evidence=dict(generation=name, list_names=[name]))
            canonical(value(listed), [name], pids, specs)
            handle, capture = await subscribe(name, 'resumed')
            owned.add(handle)
            selector = {'process': {'tag': tag}}
            for data in (b'', INPUT):
                await rpc.unary('send_input', selector | {'input': {'stdin': base64.b64encode(data).decode()}},
                                generation=name)
            await rpc.unary('close_stdin', selector, generation=name)

            async def wait(handle, capture, label, output):
                options = {} if observed.asynchronous else capture.callbacks()
                result = await calls.operation('handle.wait', partial(owned.wait, handle, **options),
                    evidence=dict(generation=name, subscription=label, handle_pid=handle.pid))
                if capture.failure is not None:
                    raise capture.failure
                assert (result.exit_code, result.error, result.stdout, result.stderr) == (0, None, output, '' if label == 'late' else 'paired stderr 中文\n')
                assert {key: ''.join(parts) for key, parts in capture.parts.items()} == {
                    'stdout': result.stdout, 'stderr': result.stderr, 'pty': ''}

            await wait(handle, capture, 'resumed', f'{len(INPUT)} {hashlib.sha256(INPUT).hexdigest()}\n')
            await removal(name)
            failure = await rpc.unary('send_signal', selector | {'signal': 'SIGNAL_SIGTERM'},
                                      generation=name, expected=ConnectError)
            not_found(failure, 'tag', tag)
            late, capture = await subscribe(name, 'late')
            owned.add(late)
            await wait(late, capture, 'late', '')
            listed = await calls.operation('commands.list', observed.sandbox.commands.list,
                request_timeout=15, evidence=dict(generation=name, list_names=[]))
            canonical(value(listed), [], pids, specs)
            marker = await calls.invoke('files.read', directory + '/' + name + '/started',
                                       user=user, request_timeout=15)
            assert marker == 'once\n', 'tag Connect or reuse replayed an earlier command'
    row['status'] = 'complete'
    return row


def comparable_tag(original):
    row = copy.deepcopy(original)
    assert row['status'] == 'complete' and set(row['pids']) == set(GENERATIONS)
    user, directory, tag = row['args']
    assert tag == TAGS[user]
    pids, specs = dict(row['pids']), definitions(directory, tag)
    for pid in pids.values():
        valid_pid(pid)
    assert len(set(pids.values())) == 2

    def replace(item, key, name):
        assert type(item[key]) is int and item[key] == pids[name], 'not the original tag generation PID'
        item[key] = '<tag-' + name + '>'

    for name in GENERATIONS:
        replace(row['pids'], name, name)
    starts, connects = [], []
    for operation in row['calls']:
        assert operation['status'] == 'complete'
        method, name = operation['method'], operation.get('generation')
        if method == 'process.start':
            starts.append(name)
            replace(operation['result'], 'pid', name)
        elif method == 'process.connect':
            connects.append((name, operation['subscription']))
            if operation['subscription'] == 'retired':
                assert name == 'first' and 'result' not in operation
                selector = operation['args'][0]['process']
                assert selector == {'pid': pids[name]}
                replace(selector, 'pid', name)
                expected = str(ConnectError(Code.NOT_FOUND, f'process with pid {pids[name]} not found'))
                assert operation['error']['message'] == expected
                operation['error']['message'] = str(ConnectError(Code.NOT_FOUND, 'process with pid <tag-first> not found'))
            else:
                assert operation['args'][0] == {'process': {'tag': tag}}
                replace(operation['result'], 'pid', name)
        elif method == 'commands.list':
            operation['result'] = canonical(operation['result'], operation['list_names'], pids, specs)
        elif method in ('handle.wait', 'handle.disconnect'):
            replace(operation, 'handle_pid', name)
        else:
            assert method in ('process.send_input', 'process.close_stdin', 'process.send_signal',
                              'commands.run', 'files.read'), 'unexpected tag operation'
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    assert starts == list(GENERATIONS)
    assert connects == [('first', 'resumed'), ('first', 'late'), ('first', 'retired'),
                        ('second', 'resumed'), ('second', 'late')]
    assert [(item['generation'], item['label']) for item in row['subscriptions']] == [
        (name, label) for name in GENERATIONS for label in ('start', 'resumed', 'late')]
    for subscription in row['subscriptions']:
        replace(subscription, 'pid', subscription['generation'])
        subscription['callbacks'] = {key: ''.join(parts) for key, parts in subscription['callbacks'].items()}
    check_observations(row['observations'], [([], None, True), ([], 'first', False), ([], 'second', False)], pids, specs)
    return row
