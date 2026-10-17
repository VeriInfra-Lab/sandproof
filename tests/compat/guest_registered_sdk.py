"""Pinned sync/async SDK -> authenticated PID/tag components in a restricted fixture.

No cloud, VM, installed daemon or fixed-Go differential result.
"""

import asyncio
import base64
import json
import sys

from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.process import process_connect, process_pb
from e2b.envd.rpc import handle_rpc_exception
from e2b.exceptions import RateLimitException
from e2b.sandbox.commands.command_handle import CommandExitException
from e2b.sandbox_sync.commands.command_handle import CommandHandle
from e2b.sandbox_async.commands.command_handle import AsyncCommandHandle
from connectrpc.code import Code
from connectrpc.errors import ConnectError

from sdk_support import isolated_sdk_environment


class Peer:
    def __init__(self, settings):
        self.asynchronous = settings['asynchronous']
        self.headers = {'x-access-token': 'token'}
        if settings['basic']:
            self.headers['authorization'] = 'Basic ' + base64.b64encode(b'fixture:').decode()
        factory, kind = ((async_client, process_connect.ProcessClient) if self.asynchronous
                         else (sync_client, process_connect.ProcessClientSync))
        self.client = factory(kind, settings['url'], ConnectionConfig())

    def call(self, method, request, headers=None):
        return getattr(self.client, method)(request, headers=self.headers | (headers or {}),
                                            timeout_ms=10000)

    async def unary(self, method, kind, payload, headers=None):
        result = self.call(method, kind.from_json(json.dumps(payload)), headers)
        return await result if self.asynchronous else result

    async def next(self, stream):
        return await anext(stream) if self.asynchronous else next(stream)

    async def close(self, stream):
        if self.asynchronous:
            await stream.aclose()
        else:
            stream.close()

    async def rejected(self, method, kind, payload, code, message, headers=None):
        try:
            if method in ('start', 'connect'):
                stream = self.call(method, kind.from_json(json.dumps(payload)), headers)
                try:
                    await self.next(stream)
                finally:
                    await self.close(stream)
            else:
                await self.unary(method, kind, payload, headers)
        except ConnectError as error:
            assert (error.code, error.message) == (code, message), repr(error)
            if code == Code.RESOURCE_EXHAUSTED:
                mapped = handle_rpc_exception(error)
                assert isinstance(mapped, RateLimitException), repr(mapped)
                assert str(mapped) == message + ': Rate limit exceeded, please try again later.'
        else:
            raise AssertionError(f'{method} unexpectedly accepted rejected request')

    async def output(self, stream, channel, expected):
        actual = bytearray()
        while len(actual) < len(expected):
            event = (await self.next(stream)).event.event
            if event.field == 'keepalive':
                continue
            assert event.field == 'data', event
            assert event.value.output.field == channel
            actual.extend(event.value.output.value)
        assert actual == expected, (actual, expected)

    async def finish(self, stream, pid, mode, tail=b''):
        events, terminal, failure = [], bytearray(), None

        def recorded_sync():
            for event in stream:
                events.append(event)
                yield event

        async def recorded_async():
            async for event in stream:
                events.append(event)
                yield event

        def unused_kill():
            raise AssertionError('wait must not send a second signal')

        async def unused_async_kill():
            unused_kill()

        try:
            if self.asynchronous:
                handle = AsyncCommandHandle(pid, unused_async_kill, recorded_async(),
                                            on_pty=terminal.extend)
                await handle.wait()
            else:
                handle = CommandHandle(pid, unused_kill, recorded_sync())
                handle.wait(on_pty=terminal.extend)
        except CommandExitException as error:
            failure = error
        finally:
            await self.close(stream)
        # Heartbeats may occur under CI scheduling delays and are passed to the
        # real handle; they are not command output or a second terminal event.
        events = [event for event in events if event.event.event.field != 'keepalive']
        assert events and events[-1].event.event.field == 'end'
        assert sum(event.event.event.field == 'end' for event in events) == 1
        assert all(event.event.event.field == 'data' for event in events[:-1])
        if mode == 'pty':
            assert all(event.event.event.value.output.field == 'pty' for event in events[:-1])
            assert terminal == tail, (terminal, tail)
        else:
            assert len(events) == 1, events
        end = events[-1].event.event.value
        if mode in ('term', 'kill'):
            status = 'signal: ' + ('terminated' if mode == 'term' else 'killed')
            assert isinstance(failure, CommandExitException) and failure.exit_code == -1
            assert (end.exit_code, end.exited, end.status, end.error) == (-1, False, status, status)
        else:
            assert failure is None, repr(failure)
            assert (end.exit_code, end.exited, end.status) == (0, True, 'exit status 0')
            assert not end.has_field('error')


async def run(settings):
    peer, mode = Peer(settings), settings['mode']
    tagged = 'tag' in settings['request']
    tag = settings['request'].get('tag')
    # The original generated RPC client may supply this explicit header. The
    # higher-level commands/PTY APIs normally send their fixed 50-second value.
    # Each rejected Start must finish before the one permitted real Start.
    for value in settings['keepalive_values']:
        for method, kind, payload in [
            ('start', process_pb.StartRequest, settings['request']),
            ('connect', process_pb.ConnectRequest, {'process': {'pid': 4294967295}}),
        ]:
            await peer.rejected(method, kind, payload, Code.INVALID_ARGUMENT,
                                'keepalive interval must be between 1 and 9223372036 seconds',
                                {'keepalive-ping-interval': value})
    request = process_pb.StartRequest.from_json(json.dumps(settings['request']))
    stream = peer.call('start', request)
    try:
        first = (await peer.next(stream)).event.event
        assert first.field == 'start' and first.value.pid > 0
        pid = first.value.pid
        listed = await peer.unary('list', process_pb.ListRequest, {})
        assert len(listed.processes) == 1
        item = listed.processes[0]
        assert item.pid == pid and item.has_field('tag') == tagged
        if tagged:
            assert item.tag == tag
        assert item.config == request.process
        assert not item.config.has_field('cwd')  # Raw request, not resolved home.
        await peer.output(stream, 'pty' if mode == 'pty' else 'stdout',
                          b'24 80\r\n' if mode == 'pty' else b'request|ready')
        if tagged:
            await peer.rejected('start', process_pb.StartRequest, settings['request'],
                                Code.ALREADY_EXISTS, f'process with tag {tag} already exists')
        else:
            await peer.rejected('start', process_pb.StartRequest, settings['request'],
                                Code.RESOURCE_EXHAUSTED, 'command registry capacity exceeded')
    finally:
        await peer.close(stream)

    selector = {'process': {'tag': tag} if tagged else {'pid': pid}}
    if tagged:
        for absent in (f'missing-{tag}', str(pid)):
            if absent == tag:
                continue
            for method, kind in [('connect', process_pb.ConnectRequest),
                                 ('send_input', process_pb.SendInputRequest),
                                 ('close_stdin', process_pb.CloseStdinRequest),
                                 ('send_signal', process_pb.SendSignalRequest),
                                 ('update', process_pb.UpdateRequest)]:
                await peer.rejected(method, kind, {'process': {'tag': absent}},
                                    Code.NOT_FOUND, f'process with tag {absent} not found')
    # Native SDK calls decode real Rust failure responses before a later valid
    # reconnect/control. They neither close stdin nor substitute a signal.
    missing = {'process': {'pid': 4294967295}}
    for method, kind in [('connect', process_pb.ConnectRequest),
                         ('send_input', process_pb.SendInputRequest),
                         ('send_signal', process_pb.SendSignalRequest)]:
        await peer.rejected(method, kind, missing, Code.NOT_FOUND,
                            'process with pid 4294967295 not found')
    for method, kind in [('connect', process_pb.ConnectRequest),
                         ('send_input', process_pb.SendInputRequest)]:
        await peer.rejected(method, kind, {}, Code.UNIMPLEMENTED,
                            'invalid input type *process.ProcessSelector')
    await peer.rejected('send_input', process_pb.SendInputRequest, selector,
                        Code.UNIMPLEMENTED, 'invalid input type <nil>')
    await peer.rejected('send_signal', process_pb.SendSignalRequest, selector,
                        Code.UNIMPLEMENTED, 'invalid signal: SIGNAL_UNSPECIFIED')
    if mode == 'pty':
        await peer.rejected('send_input', process_pb.SendInputRequest,
                            selector | {'input': {'stdin': ''}}, Code.INTERNAL,
                            'error writing to stdin: tty assigned to process — input should be written to the pty, not the stdin')
        await peer.rejected('close_stdin', process_pb.CloseStdinRequest, selector,
                            Code.UNKNOWN,
                            'error closing stdin: cannot close stdin for PTY process — send Ctrl+D (0x04) instead')
    else:
        await peer.rejected('send_input', process_pb.SendInputRequest,
                            selector | {'input': {'pty': ''}}, Code.INTERNAL,
                            'error writing to tty: tty not assigned to process — input should be written to the stdin, not the tty')
        await peer.rejected('update', process_pb.UpdateRequest,
                            selector | {'pty': {}}, Code.INTERNAL,
                            'error resizing tty: tty not assigned to process')
    if mode in ('term', 'kill'):
        # The preselected command waits on its private FIFO, so closing stdin
        # cannot race its exit. Repeated close is accepted before both writes.
        for _ in range(2):
            await peer.unary('close_stdin', process_pb.CloseStdinRequest, selector)
        for value in ('', base64.b64encode(b'not-delivered').decode()):
            await peer.rejected('send_input', process_pb.SendInputRequest,
                                selector | {'input': {'stdin': value}}, Code.INTERNAL,
                                'error writing to stdin: stdin not enabled or closed')
    stream = peer.call('connect', process_pb.ConnectRequest.from_json(json.dumps(selector)))
    try:
        first = (await peer.next(stream)).event.event
        assert first.field == 'start' and first.value.pid == pid
        if mode in ('pipe', 'pty'):
            channel = 'pty' if mode == 'pty' else 'stdin'
            await peer.unary('send_input', process_pb.SendInputRequest,
                             selector | {'input': {channel: ''}})
            if mode == 'pty':
                await peer.unary('update', process_pb.UpdateRequest,
                                 selector | {'pty': {'size': {'rows': 30, 'cols': 100}}})
                payload = b'go\n'
            else:
                # Absent PTY is the fixed no-op Update, not a terminal creation.
                await peer.unary('update', process_pb.UpdateRequest, selector)
                payload = bytes(range(256))
            await peer.unary('send_input', process_pb.SendInputRequest,
                             selector | {'input': {channel: base64.b64encode(payload).decode()}})
            if mode == 'pipe':
                await peer.output(stream, 'stdout', payload)
                await peer.unary('close_stdin', process_pb.CloseStdinRequest, selector)
        else:
            signal = 'SIGNAL_SIGTERM' if mode == 'term' else 'SIGNAL_SIGKILL'
            await peer.unary('send_signal', process_pb.SendSignalRequest,
                             selector | {'signal': signal})
        await peer.finish(stream, pid, mode, b'30 100\r\ngot:go')
    finally:
        await peer.close(stream)

    listed = await peer.unary('list', process_pb.ListRequest, {})
    assert listed.processes == []
    if tagged:
        await peer.rejected('send_signal', process_pb.SendSignalRequest, selector,
                            Code.NOT_FOUND, f'process with tag {tag} not found')
    # A late connection receives the same terminal result, without replay/exec.
    stream = peer.call('connect', process_pb.ConnectRequest.from_json(json.dumps(selector)))
    try:
        first = (await peer.next(stream)).event.event
        assert first.field == 'start' and first.value.pid == pid
        await peer.finish(stream, pid, mode)
    finally:
        await peer.close(stream)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        asyncio.run(run(settings))
    print(f"authenticated {'tag' if 'tag' in settings['request'] else 'PID'} SDK: "
          f"{settings['mode']}, async={settings['asynchronous']} passed; "
          f"{2 * len(settings['keepalive_values'])} keepalive rejections passed; "
          f"{11 if settings['mode'] in ('term', 'kill') else 9} base operation rejections passed")
    if 'tag' in settings['request']:
        print('exact tag selection, duplicate Start and missing/live-only tag rejections passed')
    else:
        print('original full registry rejection decoded as SDK RateLimitException')
