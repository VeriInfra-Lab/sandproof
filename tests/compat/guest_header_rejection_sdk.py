"""Original SDK error decoding with a controlled late HTTP request EOF.

Only the locked encoder iterator's scheduling is changed. Its bytes, real
pyqwest transport and original ConnectError parser are unchanged. The server
releases EOF after the original response is consumed; no polling sleeps.
"""
import asyncio
import json
import socket
import sys
from functools import partial

from connectrpc import _client_async, _client_sync
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.process import process_connect, process_pb

from sdk_rejection_body import gated_async, gated_sync
from sdk_support import isolated_sdk_environment

settings = json.loads(sys.stdin.read())
original_sync = _client_sync._streaming_request_content
original_async = _client_async._streaming_request_content
events = []
control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
control.settimeout(5)
control.connect(settings['control'])


_client_sync._streaming_request_content = partial(
    gated_sync, original_sync, control, events, settings.get('partial', False))
_client_async._streaming_request_content = partial(
    gated_async, original_async, control, events, settings.get('partial', False))


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
        # 'tail' records bytes offered by the encoder, not delivery to a peer.
        assert events == expected, events


with isolated_sdk_environment():
    try:
        asyncio.run(main())
    finally:
        control.close()
