"""Pinned SDK against real Rust unary file I/O; owned temporary fixtures only."""
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

HEADERS = {'x-access-token': 'fixture-token'}


def cases(root):
    def stat(name):
        return pb.StatRequest(path=f'{root}/{name}')
    def mkdir(name):
        return pb.MakeDirRequest(path=f'{root}/{name}')
    def remove(name):
        return pb.RemoveRequest(path=f'{root}/{name}')
    return [
        ('stat', stat('file'), 'file'),
        ('stat', stat('link'), 'link'),
        ('stat', stat('broken'), 'broken'),
        ('list_dir', pb.ListDirRequest(path=f'{root}/dir', depth=2), 'list'),
        ('make_dir', mkdir('new/deep'), 'directory'),
        ('make_dir', mkdir('new/deep'), Code.ALREADY_EXISTS),
        ('make_dir', mkdir('file'), Code.INVALID_ARGUMENT),
        ('move', pb.MoveRequest(source=f'{root}/file', destination=f'{root}/new/moved'), 'moved'),
        ('stat', stat('file'), Code.NOT_FOUND),
        ('list_dir', pb.ListDirRequest(path=f'{root}/new'), 'new_list'),
        ('remove', remove('new'), 'empty'),
        ('stat', stat('new'), Code.NOT_FOUND),
        ('remove', remove('missing'), 'empty'),
        ('move', pb.MoveRequest(source=f'{root}/missing', destination=f'{root}/partial/dest'), Code.NOT_FOUND),
        ('stat', stat('partial'), 'directory'),
        ('remove', remove('partial'), 'empty'),
        ('stat', stat('link'), 'broken'),
        ('list_dir', pb.ListDirRequest(path=f'{root}/quota'), Code.RESOURCE_EXHAUSTED),
        ('stat', stat('dir/a/child'), 'child'),
        ('remove', remove('broken'), 'empty'),
    ]


def check(response, failure, expected, root):
    if isinstance(expected, Code):
        assert isinstance(failure, ConnectError) and failure.code == expected, (failure, expected)
        assert response is None
        return
    assert failure is None, repr(failure)
    if expected == 'empty':
        assert json.loads(response.to_json()) == {}
    elif expected in ('list', 'new_list'):
        names = ['a', 'child'] if expected == 'list' else ['deep', 'moved']
        assert [e.name for e in response.entries] == names
        assert all(e.path.startswith(root + '/') for e in response.entries)
    else:
        entry = response.entry
        assert entry.path.startswith(root + '/') and entry.owner and entry.group
        if expected == 'broken':
            assert entry.type == pb.FileType.UNSPECIFIED and entry.mode == 0
            assert entry.symlink_target == entry.path
        elif expected == 'directory':
            assert entry.type == pb.FileType.DIRECTORY
        else:
            assert entry.type == pb.FileType.FILE
            if expected == 'child':
                assert entry.size == 5
            else:
                assert entry.size == (4 if expected == 'link' else 6)
                assert entry.metadata == {'note': 'original'}
            if expected == 'link':
                assert entry.symlink_target == root + '/file'
                assert entry.permissions.startswith('L')


def run_sync(url, root):
    client = sync_client(clients.FilesystemClientSync, url, ConnectionConfig())
    for method, request, expected in cases(root):
        response, failure = None, None
        try:
            response = getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, expected, root)


async def run_async(url, root):
    client = async_client(clients.FilesystemClient, url, ConnectionConfig())
    for method, request, expected in cases(root):
        response, failure = None, None
        try:
            response = await getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, expected, root)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        run_sync(settings['url'], settings['root'] + '/sync')
        asyncio.run(run_async(settings['url'], settings['root'] + '/async'))
    print('guest real unary files: 40 sync/async scenarios passed; no Go HTTP service or VM')
