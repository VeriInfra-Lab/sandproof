"""Fixed sync/async SDK against Rust file codec/context and synthetic attributes.

No file is opened or changed; this is not the full service or a Go comparison.
"""
import asyncio
import json
import sys

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.filesystem import filesystem_connect as clients, filesystem_pb as pb
from sdk_support import isolated_sdk_environment

HEADERS = {'authorization': 'Basic Ym9iOg==', 'x-access-token': 'fixture-token'}


def cases():
    return [
        ('stat', pb.StatRequest(path='file'), 'file'),
        ('stat', pb.StatRequest(path='large'), 'large'),
        ('stat', pb.StatRequest(path='link'), 'link'),
        ('make_dir', pb.MakeDirRequest(path='~/new'), 'new'),
        ('list_dir', pb.ListDirRequest(path='dir', depth=2**32-1), 'dir'),
        ('list_dir', pb.ListDirRequest(path='empty'), 'empty'),
        ('move', pb.MoveRequest(source='file', destination='moved'), 'moved'),
        ('remove', pb.RemoveRequest(path='moved'), 'remove'),
        ('stat', pb.StatRequest(path='budget'), 'budget'),
    ]


def check(response, error, name):
    if name == 'budget':
        assert isinstance(error, ConnectError), repr(error)
        assert error.code == Code.RESOURCE_EXHAUSTED
        assert response is None
        return
    assert error is None, repr(error)
    if name == 'remove':
        assert json.loads(response.to_json()) == {}
        return
    if name == 'empty':
        assert response.entries == []
        return
    entry = response.entries[0] if name == 'dir' else response.entry
    assert entry.path == '/home/bob/' + name
    assert entry.name == name and entry.owner == 'bob' and entry.group == 'fixture-group'
    assert entry.size == (2**63-1 if name == 'large' else 4)
    assert entry.metadata == {'note': '雪\n"\\'}
    if name == 'link':
        assert entry.type == pb.FileType.UNSPECIFIED
        assert entry.has_field('symlink_target') and entry.symlink_target == ''
        assert entry.modified_time is None and entry.mode == 0
    else:
        expected_type = pb.FileType.DIRECTORY if name == 'new' else pb.FileType.FILE
        assert entry.type == expected_type and entry.mode == 0o644
        assert not entry.has_field('symlink_target')
        assert entry.modified_time.seconds == -1 and entry.modified_time.nanos == 123456789


def run_sync(url):
    client = sync_client(clients.FilesystemClientSync, url, ConnectionConfig())
    for method, request, name in cases():
        response, failure = None, None
        try:
            response = getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, name)


async def run_async(url):
    client = async_client(clients.FilesystemClient, url, ConnectionConfig())
    for method, request, name in cases():
        response, failure = None, None
        try:
            response = await getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, name)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        run_sync(settings['url'])
        asyncio.run(run_async(settings['url']))
    print('guest file codec/context: 18 sync/async scenarios passed (synthetic attributes; no file I/O)')
