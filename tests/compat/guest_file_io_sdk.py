"""Pinned SDK against real Rust unary file I/O; owned temporary fixtures only."""
import asyncio
import base64
import json
import sys

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from connectrpc.compression.gzip import GzipCompression
from e2b.connection_config import ConnectionConfig
from e2b.envd.client_async import create_rpc_client as async_client
from e2b.envd.client_sync import create_rpc_client as sync_client
from e2b.envd.filesystem import filesystem_connect as clients, filesystem_pb as pb
from sdk_support import isolated_sdk_environment

HEADERS = {'x-access-token': 'fixture-token'}
TOO_LONG_TIMEOUT = 10_000_000_000


def cases(root):
    def stat(name):
        return pb.StatRequest(path=f'{root}/{name}')
    def mkdir(name):
        return pb.MakeDirRequest(path=f'{root}/{name}')
    def remove(name):
        return pb.RemoveRequest(path=f'{root}/{name}')
    return [
        ('stat', stat('file'), 'file'),
        ('stat', pb.StatRequest(path='~sandproof-other-311'), Code.INVALID_ARGUMENT),
        ('move', pb.MoveRequest(source='~sandproof-other-311', destination=f'{root}/path-refused/dest'), Code.INVALID_ARGUMENT),
        ('remove', remove('dir/.'), Code.INTERNAL),
        ('remove', remove('file/child'), Code.INTERNAL),
        ('list_dir', pb.ListDirRequest(path=f'{root}/missing'), Code.NOT_FOUND),
        ('list_dir', pb.ListDirRequest(path=f'{root}/file'), Code.INVALID_ARGUMENT),
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


def check(response, failure, expected, root, method=None, request=None, username=None):
    if isinstance(expected, Code):
        assert isinstance(failure, ConnectError) and failure.code == expected, (failure, expected)
        assert response is None
        if expected != Code.RESOURCE_EXHAUSTED:
            if ((method == 'stat' and request.path.startswith('~')) or
                    (method == 'move' and request.source.startswith('~'))):
                message = f"failed to expand path '' for user '{username}': cannot expand user-specific home dir"
            elif method == 'stat':
                message = f'file not found: lstat {request.path}: no such file or directory'
            elif method == 'move':
                message = f'source file not found: rename {request.source} {request.destination}: no such file or directory'
            elif method == 'list_dir':
                message = (f'path not found: lstat {request.path}: no such file or directory'
                           if expected == Code.NOT_FOUND else f'path is not a directory: {request.path}')
            elif method == 'remove':
                detail = (f'RemoveAll {request.path}: invalid argument' if request.path.endswith('/.')
                          else f'unlinkat {request.path}: not a directory')
                message = f'error removing file or directory: {detail}'
            elif expected == Code.ALREADY_EXISTS:
                message = f'directory already exists: {request.path}'
            else:
                message = f'path already exists but it is not a directory: {request.path}'
            assert failure.message == message, (failure.message, message)
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


def authorization(username):
    value = base64.b64encode((username + ':ignored-password').encode()).decode()
    return {**HEADERS, 'authorization': 'Basic ' + value}


def account_cases(settings):
    for username in ('', settings['missing_user'], "sandproof'<>&"):
        yield settings['url'], authorization(username), f"invalid username: '{username}'"
    username = settings['missing_user']
    yield settings['default_url'], HEADERS, f"invalid default user: '{username}'"


def account_failure(error, message):
    assert error.code == Code.UNAUTHENTICATED, error
    assert error.message == message, (error.message, message)


def timeout_failure(error):
    assert error.code == Code.INVALID_ARGUMENT, error
    assert error.message == 'parse timeout: "10000000000" has >10 digits', error.message


def accepting_gzip(client_cls):
    """Use the generated client option; per-call compression headers are overwritten."""
    def create(url, **kwargs):
        kwargs['accept_compression'] = [GzipCompression()]
        return client_cls(url, **kwargs)
    return create


def run_sync(settings, root):
    url = settings['url']
    client = sync_client(clients.FilesystemClientSync, url, ConnectionConfig())
    for method, request, expected in cases(root):
        response, failure = None, None
        try:
            response = getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, expected, root, method, request, settings['username'])

    response = client.stat(pb.StatRequest(path=root + '/dir/a/child'), headers=authorization(settings['username']), timeout_ms=5000)
    check(response, None, 'child', root)
    with sync_client(accepting_gzip(clients.FilesystemClientSync), url, ConnectionConfig()) as gzip_client:
        for path, expected in [('dir/a/child', 'child'), ('missing', Code.NOT_FOUND)]:
            request = pb.StatRequest(path=f'{root}/{path}')
            response, failure = None, None
            try:
                response = gzip_client.stat(request, headers=HEADERS, timeout_ms=5000)
            except ConnectError as error:
                failure = error
            check(response, failure, expected, root, 'stat', request, settings['username'])
    for target, headers, message in account_cases(settings):
        selected = sync_client(clients.FilesystemClientSync, target, ConnectionConfig())
        try:
            selected.make_dir(pb.MakeDirRequest(path=root + '/auth-refused'), headers=headers,
                              timeout_ms=TOO_LONG_TIMEOUT if target == url else 5000)
        except ConnectError as error:
            account_failure(error, message)
        else:
            raise AssertionError('unknown account was accepted')

    try:
        client.make_dir(pb.MakeDirRequest(path=root + '/timeout-refused'), headers=HEADERS, timeout_ms=TOO_LONG_TIMEOUT)
    except ConnectError as error:
        timeout_failure(error)
    else:
        raise AssertionError('oversized Connect timeout was accepted')


async def run_async(settings, root):
    url = settings['url']
    client = async_client(clients.FilesystemClient, url, ConnectionConfig())
    for method, request, expected in cases(root):
        response, failure = None, None
        try:
            response = await getattr(client, method)(request, headers=HEADERS, timeout_ms=5000)
        except ConnectError as error:
            failure = error
        check(response, failure, expected, root, method, request, settings['username'])

    response = await client.stat(pb.StatRequest(path=root + '/dir/a/child'), headers=authorization(settings['username']), timeout_ms=5000)
    check(response, None, 'child', root)
    async with async_client(accepting_gzip(clients.FilesystemClient), url, ConnectionConfig()) as gzip_client:
        for path, expected in [('dir/a/child', 'child'), ('missing', Code.NOT_FOUND)]:
            request = pb.StatRequest(path=f'{root}/{path}')
            response, failure = None, None
            try:
                response = await gzip_client.stat(request, headers=HEADERS, timeout_ms=5000)
            except ConnectError as error:
                failure = error
            check(response, failure, expected, root, 'stat', request, settings['username'])
    for target, headers, message in account_cases(settings):
        selected = async_client(clients.FilesystemClient, target, ConnectionConfig())
        try:
            await selected.make_dir(pb.MakeDirRequest(path=root + '/auth-refused'), headers=headers,
                                    timeout_ms=TOO_LONG_TIMEOUT if target == url else 5000)
        except ConnectError as error:
            account_failure(error, message)
        else:
            raise AssertionError('unknown account was accepted')

    try:
        await client.make_dir(pb.MakeDirRequest(path=root + '/timeout-refused'), headers=HEADERS, timeout_ms=TOO_LONG_TIMEOUT)
    except ConnectError as error:
        timeout_failure(error)
    else:
        raise AssertionError('oversized Connect timeout was accepted')


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        run_sync(settings, settings['root'] + '/sync')
        asyncio.run(run_async(settings, settings['root'] + '/async'))
    print('guest real unary files: 56 sync/async file scenarios (including 4 gzip negotiation cases) + 10 account scenarios + 2 timeout rejections passed; no Go HTTP service or VM')
