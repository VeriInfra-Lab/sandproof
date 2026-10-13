"""Original sync/async SDK against owned Rust raw/multipart file components.

The Rust harness owns all paths. This is not complete envd or KVM
acceptance; no production/cloud endpoint or credential is used.
"""
import asyncio
import gzip
from io import BytesIO, StringIO
import json
import sys
import httpx

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import FileNotFoundException, InvalidArgumentException
from sdk_support import isolated_sdk_environment


def arguments(url):
    return dict(api_url=url, sandbox_url=url, api_key='synthetic-local-only',
                domain='example.invalid', debug=False, request_timeout=10, retries=0)


def payloads():
    return [('empty', b''), ('binary', b'\x00\xffA\r\n'),
            ('large', bytes(range(256)) * 12289)]


def compressed_requests(root):
    """Fixed SDK has no upload-compression option; construct HTTP cases explicitly.

    Original sync/async SDK reads verify each resulting file. This does not
    pretend the compressed POSTs are unmodified SDK write calls.
    """
    raw = f'{root}/gzip-raw'
    large = bytes(range(256)) * 4097
    form = b''.join(
        b'--G\r\nContent-Disposition: form-data; name="file"; filename="'
        + path.encode() + b'"\r\n\r\n' + data + b'\r\n'
        for path, data in [(f'{root}/gzip-a', b'\x00\xff'),
                           (f'{root}/gzip-\u96ea', b'snow')]) + b'--G--\r\n'
    damaged = bytearray(gzip.compress(b'written-prefix', mtime=0))
    damaged[-8] ^= 1
    headers = {'x-access-token': 'fixture-token', 'content-encoding': 'gzip',
               'content-type': 'application/octet-stream', 'x-metadata-note': 'gzip'}
    return [
        (raw, headers, gzip.compress(large, mtime=0), 200, [(raw, large)]),
        (None, {**headers, 'content-type': 'multipart/form-data; boundary=G'},
         gzip.compress(form, mtime=0), 200,
         [(f'{root}/gzip-a', b'\x00\xff'), (f'{root}/gzip-\u96ea', b'snow')]),
        (raw, headers, b'invalid gzip header', 400, [(raw, large)]),
        (f'{root}/gzip-partial', headers, bytes(damaged), 500,
         [(f'{root}/gzip-partial', b'written-prefix')]),
    ]


def compressed_sync(sandbox, url, root):
    with httpx.Client(trust_env=False, timeout=10) as client:
        for path, headers, data, status, expected in compressed_requests(root):
            response = client.post(f'{url}/files', params={} if path is None else {'path': path},
                                   headers=headers, content=data)
            assert response.status_code == status, response.text
            if status == 200:
                assert all(entry['metadata'] == {'note': 'gzip'} for entry in response.json())
            for filename, value in expected:
                assert bytes(sandbox.files.read(filename, format='bytes')) == value


async def compressed_async(sandbox, url, root):
    async with httpx.AsyncClient(trust_env=False, timeout=10) as client:
        for path, headers, data, status, expected in compressed_requests(root):
            response = await client.post(f'{url}/files', params={} if path is None else {'path': path},
                                         headers=headers, content=data)
            assert response.status_code == status, response.text
            if status == 200:
                assert all(entry['metadata'] == {'note': 'gzip'} for entry in response.json())
            for filename, value in expected:
                assert bytes(await sandbox.files.read(filename, format='bytes')) == value


def form_payloads(root):
    large = payloads()[-1][1]
    expected = {'empty': b'', 'binary': b'\x00\xffbinary',
                'nested/雪.txt': '你好\n'.encode(), 'large': large,
                'text-io': 'snow ☃'.encode()}
    sources = [b'', expected['binary'], '你好\n', BytesIO(large), StringIO('snow ☃')]
    files = [{'path': f'{root}/{name}', 'data': data}
             for name, data in zip(expected, sources, strict=True)]
    return files, expected


def multipart_sync(sandbox, root):
    form = f'{root}/form'
    assert sandbox.files.write(f'{form}/single', 'hi').path == f'{form}/single'
    assert bytes(sandbox.files.read(f'{form}/single', format='bytes')) == b'hi'
    files, expected = form_payloads(form)
    results = sandbox.files.write_files(files, use_octet_stream=False,
                                         metadata={'note': 'multipart'})
    assert [result.path for result in results] == [file['path'] for file in files]
    assert all(result.metadata == {'note': 'multipart'} for result in results)
    for name, data in expected.items():
        assert bytes(sandbox.files.read(f'{form}/{name}', format='bytes')) == data
    with sandbox.files.read(f'{form}/large', format='stream', request_timeout=10) as stream:
        assert b''.join(stream) == expected['large']
    assert not sandbox.files.write(f'{form}/binary', b'replace').metadata
    for first, second, later in [(f'{form}/duplicate', f'{form}/duplicate', f'{form}/after-duplicate'),
                                 (f'{form}/before-directory', f'{root}/directory', f'{form}/after-directory')]:
        try:
            sandbox.files.write_files([{'path': first, 'data': b'first'},
                                       {'path': second, 'data': b'second'},
                                       {'path': later, 'data': b'never'}], use_octet_stream=False)
        except InvalidArgumentException:
            pass
        else:
            raise AssertionError('failed multipart batch reported success')
        assert bytes(sandbox.files.read(first, format='bytes')) == b'first'
        try:
            sandbox.files.read(later, format='bytes')
        except FileNotFoundException:
            pass
        else:
            raise AssertionError('multipart processed a later file after failure')
    assert sandbox.files.write_files([], use_octet_stream=False) == []


async def multipart_async(sandbox, root):
    form = f'{root}/form'
    result = await sandbox.files.write(f'{form}/single', 'hi')
    assert result.path == f'{form}/single'
    assert bytes(await sandbox.files.read(f'{form}/single', format='bytes')) == b'hi'
    files, expected = form_payloads(form)
    results = await sandbox.files.write_files(files, use_octet_stream=False,
                                               metadata={'note': 'multipart'})
    assert [result.path for result in results] == [file['path'] for file in files]
    assert all(result.metadata == {'note': 'multipart'} for result in results)
    for name, data in expected.items():
        assert bytes(await sandbox.files.read(f'{form}/{name}', format='bytes')) == data
    stream = await sandbox.files.read(f'{form}/large', format='stream', request_timeout=10)
    async with stream:
        assert b''.join([chunk async for chunk in stream]) == expected['large']
    result = await sandbox.files.write(f'{form}/binary', b'replace')
    assert not result.metadata
    for first, second, later in [(f'{form}/duplicate', f'{form}/duplicate', f'{form}/after-duplicate'),
                                 (f'{form}/before-directory', f'{root}/directory', f'{form}/after-directory')]:
        try:
            await sandbox.files.write_files([{'path': first, 'data': b'first'},
                                             {'path': second, 'data': b'second'},
                                             {'path': later, 'data': b'never'}], use_octet_stream=False)
        except InvalidArgumentException:
            pass
        else:
            raise AssertionError('failed multipart batch reported success')
        assert bytes(await sandbox.files.read(first, format='bytes')) == b'first'
        try:
            await sandbox.files.read(later, format='bytes')
        except FileNotFoundException:
            pass
        else:
            raise AssertionError('multipart processed a later file after failure')
    assert await sandbox.files.write_files([], use_octet_stream=False) == []


def run_sync(url, root):
    sandbox = Sandbox.connect('probe-sandbox', **arguments(url))
    for name, data in payloads():
        path = f'{root}/{name}'
        source = BytesIO(data) if name == 'large' else data
        result = sandbox.files.write(path, source, use_octet_stream=True)
        assert result.path == path and result.name == name
        assert bytes(sandbox.files.read(path, format='bytes')) == data
        with sandbox.files.read(path, format='stream', request_timeout=10) as stream:
            assert b''.join(stream) == data
    path = f'{root}/metadata'
    assert sandbox.files.write(path, b'old', use_octet_stream=True,
                               metadata={'note': 'original'}).metadata == {'note': 'original'}
    assert not sandbox.files.write(path, b'new', use_octet_stream=True).metadata
    assert bytes(sandbox.files.read(path, format='bytes')) == b'new'
    try:
        sandbox.files.write_files([{'path': f'{root}/partial', 'data': b'first'},
                                   {'path': f'{root}/directory', 'data': b'fail'}],
                                  use_octet_stream=True)
    except InvalidArgumentException:
        pass
    else:
        raise AssertionError('directory upload succeeded')
    assert bytes(sandbox.files.read(f'{root}/partial', format='bytes')) == b'first'
    try:
        sandbox.files.read(f'{root}/missing', format='bytes')
    except FileNotFoundException:
        pass
    else:
        raise AssertionError('missing download succeeded')
    try:
        sandbox.files.read(f'{root}/directory', format='bytes')
    except InvalidArgumentException:
        pass
    else:
        raise AssertionError('directory download succeeded')
    multipart_sync(sandbox, root)
    compressed_sync(sandbox, url, root)


async def run_async(url, root):
    sandbox = await AsyncSandbox.connect('probe-sandbox', **arguments(url))
    for name, data in payloads():
        path = f'{root}/{name}'
        source = BytesIO(data) if name == 'large' else data
        result = await sandbox.files.write(path, source, use_octet_stream=True)
        assert result.path == path and result.name == name
        assert bytes(await sandbox.files.read(path, format='bytes')) == data
        stream = await sandbox.files.read(path, format='stream', request_timeout=10)
        async with stream:
            assert b''.join([chunk async for chunk in stream]) == data
    path = f'{root}/metadata'
    result = await sandbox.files.write(path, b'old', use_octet_stream=True,
                                       metadata={'note': 'original'})
    assert result.metadata == {'note': 'original'}
    result = await sandbox.files.write(path, b'new', use_octet_stream=True)
    assert not result.metadata
    assert bytes(await sandbox.files.read(path, format='bytes')) == b'new'
    # Async raw batches run concurrently. Wait for a successful first request
    # explicitly, then check a failed second one; never assert atomic rollback.
    await sandbox.files.write(f'{root}/partial', b'first', use_octet_stream=True)
    try:
        await sandbox.files.write(f'{root}/directory', b'fail', use_octet_stream=True)
    except InvalidArgumentException:
        pass
    else:
        raise AssertionError('directory upload succeeded')
    assert bytes(await sandbox.files.read(f'{root}/partial', format='bytes')) == b'first'
    try:
        await sandbox.files.read(f'{root}/missing', format='bytes')
    except FileNotFoundException:
        pass
    else:
        raise AssertionError('missing download succeeded')
    try:
        await sandbox.files.read(f'{root}/directory', format='bytes')
    except InvalidArgumentException:
        pass
    else:
        raise AssertionError('directory download succeeded')
    await multipart_async(sandbox, root)
    await compressed_async(sandbox, url, root)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        run_sync(settings['url'], settings['root'] + '/sync')
        asyncio.run(run_async(settings['url'], settings['root'] + '/async'))
    print('Pinned sync/async SDK raw/multipart and synthetic gzip upload with SDK readback passed (test routes only).')
