"""Observe approved tag differences using the same original SDK RPC inputs.

Fixed Go may miss a live tag and accepts duplicates. Rust must find the exact
live tag and reject a duplicate before execution. Raw outcomes stay separate.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import asyncio
from contextlib import AsyncExitStack
from functools import partial
import hashlib
import shlex
from types import SimpleNamespace

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from sdk_command_input import KnownCommand, call
from sdk_list_compare import READY, Capture, canonical, observe_removal, read_ready
from sdk_observations import Calls, value
from sdk_tag_compare import TAGS, not_found, program
from sdk_tag_rpc import RecordedHandle, TagRpc

NAMES = ('first', 'other', 'duplicate')


def definitions(directory, user):
    tags = dict(first=TAGS[user], other=TAGS[user] + ':other', duplicate=TAGS[user])
    return {name: {'metadata': dict(tag=tags[name], cmd='/bin/bash',
        args=['-l', '-c', program(directory + '/' + name)], envs={}, cwd=directory)}
        for name in NAMES}


async def policy_case(observed, kind, user, parent):
    assert kind in ('go', 'rust')
    directory = parent + '/tag-policy'
    row = dict(method='commands.tag_policy', kind=kind, args=[user, directory],
               status='incomplete', pids={}, calls=[], observations=[], subscriptions=[])
    observed.rows.append(row)
    calls = Calls(observed.sandbox, observed.asynchronous)
    calls.rows = row['calls']
    rpc, specs, pids = TagRpc(calls, user), definitions(directory, user), row['pids']
    owners, original = {}, {}

    async def kill(pid, **kwargs):
        return await calls.operation('commands.kill', observed.sandbox.commands.kill, pid, **kwargs)

    async def snapshot(names):
        result = await calls.operation('commands.list', observed.sandbox.commands.list,
            request_timeout=15, evidence=dict(list_names=list(names)))
        canonical(value(result), names, pids, specs)

    async def removal(names, retiring=None):
        evidence = {}
        row['observations'].append(evidence)
        await observe_removal(calls, evidence, names, pids, specs,
                              retiring=retiring, baseline=retiring is None)

    async with AsyncExitStack() as stack:
        setup = ('mkdir -m 0700 ' + shlex.quote(directory)
                 + ''.join(' && mkdir -m 0700 ' + shlex.quote(directory + '/' + name) for name in NAMES))
        result = await calls.invoke('commands.run', setup, user=user, timeout=15)
        assert (result.exit_code, result.error, result.stdout, result.stderr) == (0, None, '', '')
        await removal([])

        async def start(name):
            metadata = specs[name]['metadata']
            payload = dict(tag=metadata['tag'], stdin=True,
                           process={key: part for key, part in metadata.items() if key != 'tag'})
            capture = Capture(False, True)
            entry = dict(generation=name, label='start', callbacks=capture.parts)
            row['subscriptions'].append(entry)
            handle = await rpc.stream('start', payload, capture.callbacks(), generation=name,
                label='start', expected=ConnectError if name == 'duplicate' else None,
                allow_success=name == 'duplicate')
            if isinstance(handle, ConnectError):
                assert kind == 'rust' and name == 'duplicate', 'fixed Go must accept duplicate Start'
                assert (handle.code, handle.message) == (
                    Code.ALREADY_EXISTS, f"process with tag {metadata['tag']} already exists")
                assert not handle.details
                return
            handle = RecordedHandle(calls, handle, name, 'start')
            entry['pid'] = handle.pid
            # A repeated returned PID cannot create a second cleanup authority.
            for known in owners.values():
                if handle.pid == known.pid:
                    known.add(handle)
                    raise AssertionError('Start reused an original policy PID')
            owned = await stack.enter_async_context(KnownCommand(SimpleNamespace(kill=kill), handle))
            owners[name], original[name] = owned, (handle, capture)
            pids[name] = owned.pid
            assert kind == 'go' or name != 'duplicate', 'Rust executed a duplicate tag'
            if observed.asynchronous:
                await asyncio.wait_for(capture.event.wait(), 15)
            else:
                await asyncio.wait_for(call(read_ready, handle.original, capture), 15)
            if capture.failure is not None:
                raise capture.failure
            assert capture.ready_seen and capture.data() == READY

        async def connect(name, stage):
            tag = specs[name]['metadata']['tag']
            handle = await rpc.stream('connect', {'process': {'tag': tag}}, {},
                generation=name, label=stage, seconds=20, expected=ConnectError, allow_success=True)
            if isinstance(handle, ConnectError):
                not_found(handle, 'tag', tag)
                assert kind == 'go', 'Rust missed a known live tag'
                return
            label = stage + '-' + name
            handle = RecordedHandle(calls, handle, name, label)
            eligible = [key for key in owners if specs[key]['metadata']['tag'] == tag]
            selected = next((key for key in eligible if pids[key] == handle.pid), name)
            row['subscriptions'].append(dict(generation=name, label=label, pid=handle.pid))
            # add retains even an unexpected subscription before checking PID;
            # cleanup may signal only each original Start, never this reply.
            owners[selected].add(handle)
            await owners[selected].disconnect(handle)

        for name in NAMES[:2]:
            await start(name)
        await snapshot(NAMES[:2])
        for name in NAMES[:2]:
            await connect(name, 'distinct')
        await start('duplicate')
        live = list(owners)
        await snapshot(live)
        for name in NAMES[:2]:
            await connect(name, 'after-duplicate')
        # PID controls target each returned original command regardless of Go's
        # tag traversal. No ambiguous tag is used to finish or clean up a child.
        for name, owned in owners.items():
            await rpc.unary('close_stdin', {'process': {'pid': owned.pid}}, generation=name)
            handle, capture = original[name]
            options = {} if observed.asynchronous else capture.callbacks()
            result = await calls.operation('handle.wait', partial(owned.wait, handle, **options),
                evidence=dict(generation=name, subscription='start', handle_pid=owned.pid))
            if capture.failure is not None:
                raise capture.failure
            assert (result.exit_code, result.error, result.stdout, result.stderr) == (
                0, None, READY.decode() + f'0 {hashlib.sha256(b"").hexdigest()}\n', 'paired stderr 中文\n')
            assert {channel: ''.join(parts) for channel, parts in capture.parts.items()} == {
                'stdout': result.stdout, 'stderr': result.stderr, 'pty': ''}
            live.remove(name)
            await removal(live, name)
        await snapshot([])
        for name in NAMES:
            path = directory + '/' + name + '/started'
            if name == 'duplicate' and kind == 'rust':
                assert await calls.invoke('files.exists', path, user=user, request_timeout=15) is False
            else:
                assert await calls.invoke('files.read', path, user=user, request_timeout=15) == 'once\n'
    row['status'] = 'complete'
    return row


async def policy_cases(sandbox, inputs, asynchronous, kind, rows):
    observed = Calls(sandbox, asynchronous)
    observed.rows = rows
    for user in inputs['users']:
        await policy_case(observed, kind, user, inputs['directory'] + '/' + user)
