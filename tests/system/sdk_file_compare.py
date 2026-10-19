"""File-user/path failures and partial batches in the same-input guest probe."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import asyncio
import copy
import shlex
import time

from e2b.exceptions import AuthenticationException, FileNotFoundException, InvalidArgumentException
from e2b.sandbox.filesystem.filesystem import FileType
from sdk_observations import Calls, value

DATA = bytes(range(256)) * 3
OBSERVATION_SECONDS = 15
OBSERVATION_ROUNDS = 128


async def command(calls, text, *, user='root'):
    result = await calls.invoke('commands.run', text, user=user, timeout=15)
    assert result.exit_code == 0 and result.error is None and not result.stderr
    return result.stdout


async def freeze(calls, path, mtime):
    # Only existing entries under this case's own tree are touched. Timestamps
    # remain in every compared EntryInfo; they are never deleted or rewritten.
    output = await command(calls, 'find ' + shlex.quote(path)
        + ' -exec touch -h -d @' + str(mtime) + ' {} +')
    assert output == ''


async def info(calls, path, mtime, *, user='root', owner=None, kind=None, size=None, mode=None):
    result = await calls.invoke('files.get_info', path, user=user, request_timeout=15)
    assert result.modified_time.timestamp() == mtime
    if path.startswith('/'):
        assert result.path == path
    if owner is not None:
        assert result.owner == owner
    if kind is not None:
        assert result.type == kind
    if size is not None:
        assert result.size == size
    if mode is not None:
        assert result.mode & 0o777 == mode
    return result


async def healthy(calls, directory):
    path = directory + '/healthy'
    await calls.invoke('files.write', path, 'healthy', user='ubuntu', request_timeout=15)
    result = await calls.invoke('files.read', path, user='ubuntu', request_timeout=15)
    assert result == 'healthy'
    identity = await command(calls, 'id -un', user='ubuntu')
    assert identity == 'ubuntu\n'


async def file_cases(calls, inputs):
    directory, mtime = inputs['directory'] + '/files-context', inputs['mtime']
    private, secret = directory + '/private', directory + '/private/secret'
    output = await command(calls, 'mkdir -m 0777 ' + shlex.quote(directory)
        + ' && mkdir -m 0700 ' + shlex.quote(private)
        + ' && printf original > ' + shlex.quote(secret)
        + ' && chmod 0600 ' + shlex.quote(secret))
    assert output == ''
    denied = 'if test -r ' + shlex.quote(secret) + '; then exit 41; fi; id -un'
    identity = await command(calls, denied, user='ubuntu')
    assert identity == 'ubuntu\n'
    original = await calls.invoke('files.read', secret, user='ubuntu', request_timeout=15)
    assert original == 'original'
    await freeze(calls, private, mtime)
    await info(calls, secret, mtime, user='ubuntu', owner='root', kind=FileType.FILE, size=8, mode=0o600)
    listed = await calls.invoke('files.list', private, user='ubuntu', request_timeout=15)
    assert [entry.path for entry in listed] == [secret]
    assert all(entry.modified_time.timestamp() == mtime for entry in listed)
    await calls.invoke('files.write', secret, 'overwritten', user='ubuntu', request_timeout=15)
    await freeze(calls, private, mtime)
    await info(calls, secret, mtime, owner='ubuntu', kind=FileType.FILE, size=11, mode=0o600)
    overwritten = await calls.invoke('files.read', secret, user='root', request_timeout=15)
    assert overwritten == 'overwritten'
    made = await calls.invoke('files.make_dir', private + '/child', user='ubuntu', request_timeout=15)
    assert made is True
    await freeze(calls, private, mtime)
    await info(calls, private + '/child', mtime, owner='ubuntu', kind=FileType.DIR)
    await info(calls, private, mtime, owner='root', kind=FileType.DIR, mode=0o700)
    identity = await command(calls, denied, user='ubuntu')
    assert identity == 'ubuntu\n'
    home_name = inputs['directory'].rsplit('/', 1)[-1] + '-home'
    for user, home in (('root', '/root'), ('ubuntu', '/home/ubuntu')):
        relative = home_name + '/nested/item'
        for path in (relative, '~/' + relative):
            written = await calls.invoke('files.write', path, 'home:' + user, user=user, request_timeout=15)
            assert written.path == home + '/' + relative
            content = await calls.invoke('files.read', path, user=user, request_timeout=15)
            assert content == 'home:' + user
            await freeze(calls, home + '/' + home_name, mtime)
            result = await info(calls, path, mtime, user=user, owner=user, kind=FileType.FILE)
            assert result.path == written.path
    absent = directory + '/unknown-user'
    for name, args in [('read', (secret,)), ('write', (absent, 'unexpected')),
                       ('get_info', (secret,)), ('list', (private,)),
                       ('make_dir', (absent,)), ('exists', (secret,))]:
        await calls.invoke('files.' + name, *args, user=inputs['unknown_user'], request_timeout=15,
                           expected=AuthenticationException)
        exists = await calls.invoke('files.exists', absent, user='root', request_timeout=15)
        assert exists is False
        await healthy(calls, directory)
    existing, missing = directory + '/ordinary', directory + '/absent'
    await calls.invoke('files.write', existing, 'unchanged', user='root', request_timeout=15)
    errors = [('read', missing, FileNotFoundException), ('get_info', missing, FileNotFoundException),
              ('list', missing, FileNotFoundException), ('read', directory, InvalidArgumentException),
              ('list', existing, InvalidArgumentException), ('make_dir', existing, InvalidArgumentException)]
    for user in inputs['users']:
        exists = await calls.invoke('files.exists', missing, user=user, request_timeout=15)
        assert exists is False
        for name, path, expected in errors:
            await calls.invoke('files.' + name, path, user=user, request_timeout=15, expected=expected)
            await healthy(calls, directory)
        for octet in (False, True):
            await calls.invoke('files.write', directory, 'unexpected', user=user, request_timeout=15,
                               use_octet_stream=octet, expected=InvalidArgumentException)
        await calls.invoke('files.write', '~somebody/item', 'unexpected', user=user, request_timeout=15,
                           expected=InvalidArgumentException)
        await multipart_case(calls, directory + '/multipart-' + user, user, mtime)
    content = await calls.invoke('files.read', existing, user='root', request_timeout=15)
    assert content == 'unchanged'


def entries(directory):
    # In-memory bytes retain the SDK's whole-request timeout in both transports.
    # File-like uploads deliberately have no client deadline in the fixed SDK.
    return [dict(path=directory + '/' + name, data=DATA) for name in ('first', 'directory', 'last')]


async def snapshot(calls, directory, user, mtime, last_written):
    await freeze(calls, directory, mtime)
    for name in ('first', 'last') if last_written else ('first',):
        path = directory + '/' + name
        result = await info(calls, path, mtime, owner=user, kind=FileType.FILE, size=len(DATA))
        assert result.path == path
        data = await calls.invoke('files.read', path, format='bytes', user='root', request_timeout=15)
        assert data == DATA
    exists = await calls.invoke('files.exists', directory + '/last', user='root', request_timeout=15)
    assert exists is last_written
    await info(calls, directory + '/directory', mtime, owner=user, kind=FileType.DIR)
    await healthy(calls, directory)


async def multipart_case(calls, directory, user, mtime):
    made = await calls.invoke('files.make_dir', directory + '/directory', user=user, request_timeout=15)
    assert made is True
    await calls.invoke('files.write_files', entries(directory), user=user, use_octet_stream=False,
                       request_timeout=15, expected=InvalidArgumentException)
    await snapshot(calls, directory, user, mtime, False)


async def settle(calls, directory, user, asynchronous, evidence, *, clock=time.monotonic, pause=asyncio.sleep):
    expected = [directory + '/first'] + ([directory + '/last'] if asynchronous else [])
    evidence.update(expected_files=expected, status='incomplete', calls=[])
    observations = Calls(calls.sandbox, calls.asynchronous)
    observations.rows = evidence['calls']
    deadline, sizes, complete = clock() + OBSERVATION_SECONDS, {}, set()
    def remaining():
        seconds = deadline - clock()
        assert seconds > 0, 'partial batch observation deadline'
        return seconds
    for _ in range(OBSERVATION_ROUNDS):
        for path in expected:
            if path in complete:
                continue
            result = await observations.invoke('files.get_info', path, user='root', request_timeout=remaining(),
                expected=FileNotFoundException, allow_success=True)
            remaining()
            if type(result) is FileNotFoundException:
                assert path not in sizes, 'upload path disappeared after becoming visible'
                continue
            assert result.path == path and result.type == FileType.FILE
            assert type(result.size) is int and sizes.get(path, 0) <= result.size <= len(DATA)
            sizes[path] = result.size
            if result.size == len(DATA) and result.owner == user:
                data = await observations.invoke('files.read', path, format='bytes', user='root',
                                                 request_timeout=remaining())
                remaining()
                assert data == DATA, 'completed partial batch bytes differ'
                complete.add(path)
        if len(complete) == len(expected):
            evidence['status'] = 'complete'
            return
        await pause(min(0.02, remaining()))
    raise AssertionError('partial batch observation count exceeded')


async def sdk_file_cases(sandbox, inputs, asynchronous, rows):
    # Octet batching is intentionally compared Go/Rust within each SDK mode:
    # the original sync client is sequential, the original async client gathers
    # independent requests whose replies may still be pending after an error.
    for user in inputs['users']:
        directory = inputs['directory'] + '/octet-' + user
        row = dict(method='files.octet_batch', args=[user, directory], asynchronous=asynchronous,
                   status='incomplete', calls=[], settlement={})
        rows.append(row)
        calls = Calls(sandbox, asynchronous)
        calls.rows = row['calls']
        made = await calls.invoke('files.make_dir', directory + '/directory', user=user, request_timeout=15)
        assert made is True
        await calls.invoke('files.write_files', entries(directory), user=user, use_octet_stream=True,
                           request_timeout=15, expected=InvalidArgumentException)
        await settle(calls, directory, user, asynchronous, row['settlement'])
        await snapshot(calls, directory, user, inputs['mtime'], asynchronous)
        row['status'] = 'complete'


def comparable_batch(original):
    row = copy.deepcopy(original)
    assert row['status'] == 'complete' and type(row['asynchronous']) is bool
    settlement = row['settlement']
    directory = row['args'][1]
    expected = [directory + '/first'] + ([directory + '/last'] if row['asynchronous'] else [])
    assert settlement['status'] == 'complete' and settlement['expected_files'] == expected
    samples = settlement['calls']
    assert 0 < len(samples) <= OBSERVATION_ROUNDS * len(expected) * 2
    sizes, complete, pending = {}, set(), None
    previous_timeout = OBSERVATION_SECONDS
    for sample in samples:
        assert sample['status'] == 'complete' and len(sample['args']) == 1
        path = sample['args'][0]
        assert path in expected and path not in complete
        timeout = sample['kwargs']['request_timeout']
        assert type(timeout) in (int, float) and 0 < timeout <= previous_timeout
        previous_timeout = timeout
        kwargs = dict(user='root', request_timeout=timeout)
        if pending is not None:
            assert path == pending and sample['method'] == 'files.read'
            kwargs['format'] = 'bytes'
            assert 'error' not in sample and sample['result'] == value(DATA)
            complete.add(path)
            pending = None
        else:
            assert sample['method'] == 'files.get_info'
            if 'error' in sample:
                assert 'result' not in sample and path not in sizes
                assert sample['error']['class'] == 'e2b.exceptions.FileNotFoundException'
                assert type(sample['error']['message']) is str
            else:
                result = sample['result']
                assert result['path'] == path and result['type'] == FileType.FILE.value
                size = result['size']
                assert type(size) is int and sizes.get(path, 0) <= size <= len(DATA)
                sizes[path] = size
                if size == len(DATA) and result['owner'] == row['args'][0]:
                    pending = path
        assert sample['kwargs'] == kwargs
    assert pending is None and complete == set(expected)
    # These bounded read-only samples establish completion of the observable
    # files, not acknowledgement of each independent SDK upload. Their times,
    # counts and transient metadata are retained in evidence, not compared as
    # if the two VMs progressed at identical speed. Final metadata/bytes and all
    # scenario calls below remain exact and have no such exclusion.
    del settlement['calls']
    for operation in row['calls']:
        assert operation['status'] == 'complete'
        if 'callbacks' in operation:
            operation['callbacks'] = {key: ''.join(parts) for key, parts in operation['callbacks'].items()}
    return row
