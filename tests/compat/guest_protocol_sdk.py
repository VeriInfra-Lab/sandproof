"""Fixed SDK -> test-only Rust HTTP router -> real Linux command.

Invoked by the Rust sdk-contract-tests feature. Not a production guest server
or a differential run against the fixed Go envd; no VM or cloud service.
"""

import asyncio
import json
import sys

from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.process import process_connect, process_pb
from e2b.sandbox.commands.command_handle import CommandExitException
from e2b.sandbox_sync.commands.command_handle import CommandHandle
from e2b.sandbox_async.commands.command_handle import AsyncCommandHandle

from sdk_support import isolated_sdk_environment


def check(events, mode, failure, terminal):
    assert events and events[0].event.event.field == 'start'
    assert events[0].event.event.value.pid > 0
    if mode == 'truncated':
        assert str(failure) == 'Command ended without an end event', repr(failure)
        assert all(event.event.event.field != 'end' for event in events)
        return
    if mode == 'large':
        assert failure is None, repr(failure)
    else:
        assert isinstance(failure, CommandExitException), repr(failure)
        assert failure.exit_code == (17 if mode in ('binary', 'pty') else -1)
    assert events[-1].event.event.field == 'end'
    assert sum(event.event.event.field == 'start' for event in events) == 1
    assert sum(event.event.event.field == 'end' for event in events) == 1
    stdout = bytearray()
    stderr = bytearray()
    pty = bytearray()
    for event in events[1:-1]:
        assert event.event.event.field == 'data'
        output = event.event.event.value.output
        if mode == 'pty':
            assert output.field == 'pty'
            pty.extend(output.value)
        else:
            assert output.field in ('stdout', 'stderr')
            (stdout if output.field == 'stdout' else stderr).extend(output.value)
    end = events[-1].event.event.value
    if mode == 'pty':
        assert pty == terminal == b'\0\xffAerr'
        assert not stdout and not stderr
        assert (end.exit_code, end.exited, end.status, end.error) == (
            17, True, 'exit status 17', 'exit status 17')
    elif mode == 'binary':
        assert stdout == b'\0\xffA' and stderr == b'err'
        assert (end.exit_code, end.exited, end.status, end.error) == (
            17, True, 'exit status 17', 'exit status 17')
    elif mode == 'large':
        assert stdout == bytes(8192 * 257) and not stderr
        assert (end.exit_code, end.exited, end.status) == (0, True, 'exit status 0')
        assert not end.has_field('error')
        assert len(events) > 100
    elif mode == 'signal':
        assert stdout == b'ready' and not stderr
        assert (end.exit_code, end.exited, end.status, end.error) == (
            -1, False, 'signal: killed', 'signal: killed')
    else:
        raise AssertionError(mode)


def check_list(reply, shape):
    # Controlled Rust codec observations, not an active-process table.
    if shape == 'empty':
        assert reply.processes == []
    elif shape == 'absent':
        assert len(reply.processes) == 1
        item = reply.processes[0]
        assert item.config is None and item.pid == 0 and not item.has_field('tag')
    else:
        assert len(reply.processes) == 2
        first, item = reply.processes
        assert first.pid == 123 and not first.has_field('tag')
        assert first.config is not None and first.config.cmd == ''
        assert first.config.args == [] and first.config.envs == {}
        assert not first.config.has_field('cwd')
        assert item.pid == 4294967295 and item.has_field('tag') and item.tag == ''
        assert item.config.cmd == '/bin/cat'
        assert item.config.args == ['', '"\\\n中💡']
        assert item.config.envs == {'': '', 'KEY': 'value'}
        assert item.config.has_field('cwd') and item.config.cwd == ''


def run_sync(settings):
    client = sync_client(process_connect.ProcessClientSync, settings['url'], ConnectionConfig())
    for shape in ('empty', 'absent', 'present'):
        check_list(client.list(process_pb.ListRequest(), headers={'x-fixture-list': shape}, timeout_ms=10000), shape)
    request = process_pb.StartRequest.from_json(json.dumps(settings['request']))
    stream = client.start(request, timeout_ms=10000)
    first = next(stream)
    events, failure = [first], None

    def recorded():
        for event in stream:
            events.append(event)
            yield event

    def unused_kill():
        raise AssertionError('waiting must not send kill')

    handle = CommandHandle(first.event.event.value.pid, unused_kill, recorded())
    terminal = bytearray()
    try:
        handle.wait(on_pty=terminal.extend)
    except Exception as error:
        failure = error
    finally:
        stream.close()
    check(events, settings['mode'], failure, terminal)


async def run_async(settings):
    client = async_client(process_connect.ProcessClient, settings['url'], ConnectionConfig())
    for shape in ('empty', 'absent', 'present'):
        check_list(await client.list(process_pb.ListRequest(), headers={'x-fixture-list': shape}, timeout_ms=10000), shape)
    request = process_pb.StartRequest.from_json(json.dumps(settings['request']))
    stream = client.start(request, timeout_ms=10000)
    first = await anext(stream)
    events, failure = [first], None

    async def recorded():
        async for event in stream:
            events.append(event)
            yield event

    async def unused_kill():
        raise AssertionError('waiting must not send kill')

    terminal = bytearray()
    handle = AsyncCommandHandle(first.event.event.value.pid, unused_kill, recorded(), on_pty=terminal.extend)
    try:
        await handle.wait()
    except Exception as error:
        failure = error
    finally:
        await stream.aclose()
    check(events, settings['mode'], failure, terminal)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        if settings['asynchronous']:
            asyncio.run(run_async(settings))
        else:
            run_sync(settings)
    print(f"guest wire/real process: {settings['mode']}, async={settings['asynchronous']} passed; 3 synthetic List replies decoded")
