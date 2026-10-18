"""Original SDK error decoding with a controlled late request-body handoff.

Only the locked encoder iterator's scheduling is changed. Its bytes, real
pyqwest transport and original ConnectError parser are unchanged. The server
releases the producer after the original response body is consumed; no polling
sleeps. Offering a chunk is distinct from another iterator poll, HTTP EOF or
confirmed socket delivery.
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

from sdk_rejection_body import gated_async as body_gated_async, gated_sync as body_gated_sync
from sdk_support import isolated_sdk_environment

settings = {}
original_sync = _client_sync._streaming_request_content
original_async = _client_async._streaming_request_content
events = []
control = None


def gated_sync(*args):
    return body_gated_sync(original_sync, control, events, settings.get('partial', False), *args)


def gated_async(*args):
    return body_gated_async(original_async, control, events, settings.get('partial', False), *args)


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
    # Keep original transport/protocol failures intact. An observer failure in
    # finally would overwrite the error that this acceptance test must expose.
    expected = ['message', 'waiting', 'released']
    if settings.get('partial'):
        expected.append('tail')
    assert events == expected, events


def run():
    global settings, control
    settings = json.loads(sys.stdin.read())
    control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    control.settimeout(5)
    control.connect(settings['control'])
    _client_sync._streaming_request_content = gated_sync
    _client_async._streaming_request_content = gated_async
    try:
        with isolated_sdk_environment():
            asyncio.run(main())
    finally:
        _client_sync._streaming_request_content = original_sync
        _client_async._streaming_request_content = original_async
        control.close()


if __name__ == '__main__':
    run()
