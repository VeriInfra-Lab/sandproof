"""Original SDK error decoding with a controlled late HTTP request EOF.

Control encoder scheduling and fixture teardown while preserving its bytes,
real pyqwest transport and original ConnectError parser. The server releases
the encoder gate after its original response body reaches EOF; no polling sleeps.
"""
import asyncio
import json
import socket
import sys
import threading
from functools import partial

from connectrpc import _client_async, _client_sync
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.process import process_connect, process_pb

from sdk_rejection_body import finish_gate, gated_async, gated_sync
from sdk_event_loop import JoinedCallbackLoop
from sdk_support import isolated_sdk_environment

settings = {}
original_sync = _client_sync._streaming_request_content
original_async = _client_async._streaming_request_content
events = []
upload_settled = asyncio.Event()
sync_upload_settled = threading.Event()
control = None


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
    first_error = None
    try:
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
    except BaseException as error:
        first_error = error
        raise
    finally:
        try:
            if asynchronous:
                await stream.aclose()
                await asyncio.wait_for(upload_settled.wait(), 5)
            else:
                stream.close()
                assert sync_upload_settled.wait(5), 'request iterator did not settle'
            expected = ['message', 'waiting', 'released']
            if settings.get('partial'):
                expected.append('tail')
            # 'tail' records bytes offered by the encoder, not delivery to a peer.
            # Response completion and upload cancellation/resumption are distinct
            # observations. Neither the SDK error nor response aclose orders them.
            cancelled = ['message', 'waiting', 'cancelled']
            assert events == expected or (asynchronous and events == cancelled), events
            await asyncio.wait_for(finish_gate(control, cancelled=events == cancelled), 5)
        except BaseException as cleanup_error:
            if first_error is not None:
                # Keep the original SDK/protocol failure primary, while also
                # retaining the actual settlement/observation failure as cause.
                raise first_error from cleanup_error
            raise


def run():
    global settings, control, upload_settled, sync_upload_settled
    settings = json.loads(sys.stdin.read())
    events.clear()
    upload_settled = asyncio.Event()
    sync_upload_settled = threading.Event()
    control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    control.settimeout(5)
    try:
        control.connect(settings['control'])
        _client_sync._streaming_request_content = partial(
            gated_sync, original_sync, control, events, settings.get('partial', False),
            settled=sync_upload_settled)
        _client_async._streaming_request_content = partial(
            gated_async, original_async, control, events, settings.get('partial', False),
            settled=upload_settled)
        with isolated_sdk_environment():
            asyncio.run(main(), loop_factory=JoinedCallbackLoop)
    finally:
        _client_sync._streaming_request_content = original_sync
        _client_async._streaming_request_content = original_async
        control.close()


if __name__ == '__main__':
    run()
