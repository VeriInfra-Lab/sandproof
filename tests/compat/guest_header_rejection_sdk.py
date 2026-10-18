"""Original SDK error decoding with a controlled late HTTP request EOF.

Only the locked encoder iterator's scheduling is changed. Its bytes, real
pyqwest transport and original ConnectError parser are unchanged. The server
releases EOF after the original response is consumed; no polling sleeps.
"""
import asyncio
import json
import socket
import sys

from connectrpc import _client_async, _client_sync
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.process import process_connect, process_pb

from sdk_support import isolated_sdk_environment

settings = json.loads(sys.stdin.read())
original_sync = _client_sync._streaming_request_content
original_async = _client_async._streaming_request_content
events = []
control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
control.settimeout(5)
control.connect(settings['control'])


def gated_sync(*args):
    for chunk in original_sync(*args):
        events.append('message')
        yield chunk[:1] if settings.get('partial') else chunk
        events.append('waiting')
        control.sendall(b'm')
        assert control.recv(1) == b'x', 'response completion gate missing'
        events.append('released')
        if settings.get('partial'):
            yield chunk[1:]
            events.append('tail')


async def gated_async(*args):
    async for chunk in original_async(*args):
        events.append('message')
        yield chunk[:1] if settings.get('partial') else chunk
        events.append('waiting')
        control.sendall(b'm')
        assert await asyncio.to_thread(control.recv, 1) == b'x', 'response completion gate missing'
        events.append('released')
        if settings.get('partial'):
            yield chunk[1:]
            events.append('tail')


_client_sync._streaming_request_content = gated_sync
_client_async._streaming_request_content = gated_async


async def main():
    asynchronous = settings['asynchronous']
    factory, kind = ((async_client, process_connect.ProcessClient) if asynchronous else
                     (sync_client, process_connect.ProcessClientSync))
    client = factory(kind, settings['url'], ConnectionConfig())
    stream = client.start(
        process_pb.StartRequest.from_json('{"process":{"cmd":"/bin/true"}}'),
        headers={'x-access-token': 'token', 'keepalive-ping-interval': '0'},
        timeout_ms=10000,
    )
    try:
        if asynchronous:
            await anext(stream)
        else:
            next(stream)
    except ConnectError as error:
        assert (error.code, error.message) == (
            Code.INVALID_ARGUMENT,
            'keepalive interval must be between 1 and 9223372036 seconds',
        ), repr(error)
    else:
        raise AssertionError('invalid keepalive unexpectedly accepted')
    finally:
        if asynchronous:
            await stream.aclose()
        else:
            stream.close()
        expected = ['message', 'waiting', 'released']
        if settings.get('partial'):
            expected.append('tail')
        assert events == expected, events


with isolated_sdk_environment():
    try:
        asyncio.run(main())
    finally:
        control.close()
