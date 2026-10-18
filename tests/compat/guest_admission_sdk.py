"""Pinned SDK errors from the real bounded GuestHttp listener with full pools.

Generated sync/async RPC clients send shared requests. Raw file requests use
httpx and the original SDK error parser; HEAD has no body to parse. No KVM.
"""
import asyncio
import json
from pathlib import Path
import re
import sys

import httpx
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.filesystem import filesystem_connect, filesystem_pb
from e2b.envd.process import process_connect, process_pb
from e2b.envd.rpc import handle_rpc_exception
from e2b.exceptions import RateLimitException
from e2b.sandbox_sync.filesystem.filesystem import _handle_filesystem_envd_api_exception
from sdk_support import isolated_sdk_environment


async def exercise(url):
    contract = json.loads(Path(__file__).with_name('guest-service-contract.json').read_text())
    samples = {(row['service'], row['method']): row['request'] for row in contract['rpcs']}
    count = 0
    for asynchronous in (False, True):
        factory = async_client if asynchronous else sync_client
        clients = {
            'process': factory(process_connect.ProcessClient if asynchronous else
                               process_connect.ProcessClientSync, url, ConnectionConfig()),
            'filesystem': factory(filesystem_connect.FilesystemClient if asynchronous else
                                  filesystem_connect.FilesystemClientSync, url, ConnectionConfig()),
        }
        async with httpx.AsyncClient(base_url=url, timeout=5) as async_http:
            with httpx.Client(base_url=url, timeout=5) as sync_http:
                for case in contract['errors']:
                    request, expected = case['request'], case['response']
                    path = request['path']
                    if path.startswith('/files?'):
                        kwargs = dict(content=b'never' if request['method'] == 'POST' else b'')
                        response = (await async_http.request(request['method'], path, **kwargs)
                                    if asynchronous else sync_http.request(request['method'], path, **kwargs))
                        assert response.status_code == expected['status'], response
                        assert response.text == expected['body'], response.text
                        assert response.headers['connection'] == 'close'
                        if request['method'] != 'HEAD':
                            error = _handle_filesystem_envd_api_exception(response)
                            assert isinstance(error, RateLimitException), repr(error)
                            assert str(error) == 'request capacity exhausted: The requests are being rate limited.'
                    else:
                        service = 'process' if path.startswith('/process.') else 'filesystem'
                        method = path.rsplit('/', 1)[1]
                        schema = process_pb if service == 'process' else filesystem_pb
                        payload = getattr(schema, method + 'Request').from_json(
                            json.dumps(samples[(service, method)]))
                        operation = re.sub(r'(?<!^)(?=[A-Z])', '_', method).lower()
                        stream = None
                        try:
                            reply = getattr(clients[service], operation)(payload, timeout_ms=5000)
                            if method in ('Start', 'Connect'):
                                stream = reply
                                if asynchronous:
                                    await anext(stream)
                                else:
                                    next(stream)
                            elif asynchronous:
                                await reply
                        except ConnectError as error:
                            body = json.loads(expected['body'])
                            assert (error.code, error.message) == (Code.RESOURCE_EXHAUSTED, body['message']), repr(error)
                            mapped = handle_rpc_exception(error)
                            assert isinstance(mapped, RateLimitException), repr(mapped)
                            assert str(mapped) == body['message'] + ': Rate limit exceeded, please try again later.'
                        else:
                            raise AssertionError(f"accepted {case['id']}")
                        finally:
                            if stream is not None:
                                if asynchronous:
                                    await stream.aclose()
                                else:
                                    stream.close()
                    count += 1
    print(json.dumps({'capacity_error_observations': count, 'sync_and_async': True}))


if __name__ == '__main__':
    with isolated_sdk_environment():
        asyncio.run(exercise(sys.argv[1]))
