"""Original sync/async SDK against owned Rust raw/multipart file components.

The Rust harness owns all paths. This is not complete envd or KVM
acceptance; no production/cloud endpoint or credential is used.
"""
import asyncio
import gzip
from email import policy
from email.parser import BytesParser
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


def download_headers(response):
    assert response.status_code == 200
    assert response.headers['content-encoding'] == 'gzip'
    assert response.headers['content-type'] == 'application/octet-stream'
    assert response.headers['content-disposition'].startswith('inline; filename=')
    assert response.headers['vary'] == 'Accept-Encoding'
    assert 'content-length' not in response.headers


def gzip_download_sync(sandbox, url, root):
    with httpx.Client(trust_env=False, timeout=10) as client:
        for name, data in payloads():
            path = f'{root}/{name}'
            assert bytes(sandbox.files.read(path, format='bytes', gzip=True)) == data
            with sandbox.files.read(path, format='stream', gzip=True, request_timeout=10) as stream:
                assert b''.join(stream) == data
            with client.stream('GET', f'{url}/files', params={'path': path},
                               headers={'x-access-token': 'fixture-token', 'accept-encoding': 'gzip'}) as response:
                download_headers(response)
                assert gzip.decompress(b''.join(response.iter_raw())) == data
    assert sandbox.files.read(f'{root}/form/nested/雪.txt', gzip=True) == '你好\n'
    for name, exception in [('missing', FileNotFoundException), ('directory', InvalidArgumentException)]:
        try:
            sandbox.files.read(f'{root}/{name}', gzip=True)
        except exception:
            pass
        else:
            raise AssertionError('gzip download error was reported as success')


async def gzip_download_async(sandbox, url, root):
    async with httpx.AsyncClient(trust_env=False, timeout=10) as client:
        for name, data in payloads():
            path = f'{root}/{name}'
            assert bytes(await sandbox.files.read(path, format='bytes', gzip=True)) == data
            stream = await sandbox.files.read(path, format='stream', gzip=True, request_timeout=10)
            async with stream:
                assert b''.join([chunk async for chunk in stream]) == data
            async with client.stream('GET', f'{url}/files', params={'path': path},
                                     headers={'x-access-token': 'fixture-token', 'accept-encoding': 'gzip'}) as response:
                download_headers(response)
                assert gzip.decompress(b''.join([chunk async for chunk in response.aiter_raw()])) == data
    assert await sandbox.files.read(f'{root}/form/nested/雪.txt', gzip=True) == '你好\n'
    for name, exception in [('missing', FileNotFoundException), ('directory', InvalidArgumentException)]:
        try:
            await sandbox.files.read(f'{root}/{name}', gzip=True)
        except exception:
            pass
        else:
            raise AssertionError('gzip download error was reported as success')


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


def range_requests(modified):
    # Synthetic HTTP conditions, after the original SDK wrote these files.
    return [
        ('GET', 'metadata', {'range': 'bytes=0-0'}, 206, b'n'),
        ('GET', 'metadata', {'range': 'bytes=-1'}, 206, b'w'),
        ('GET', 'metadata', {'range': 'bytes=0-0,2-2'}, 206, None),
        ('GET', 'metadata', {'range': 'bytes=99-'}, 416, b'invalid range: failed to overlap\n'),
        ('GET', 'empty', {'range': 'bytes=99-'}, 200, b''),
        ('GET', 'metadata', {'if-none-match': '*'}, 304, b''),
        ('GET', 'metadata', {'if-match': '"missing"'}, 412, b''),
        ('GET', 'metadata', {'if-modified-since': modified}, 304, b''),
        ('GET', 'metadata', {'range': 'bytes=0-0', 'if-range': 'bad'}, 200, b'new'),
        ('HEAD', 'metadata', {'range': 'bytes=1-2'}, 206, b''),
        ('HEAD', 'metadata', {'range': 'bad'}, 416, b''),
    ]


def check_range(response, method, status, expected):
    assert response.status_code == status, (response.status_code, response.content)
    assert 'content-encoding' not in response.headers
    if expected is not None:
        assert response.content == expected
    else:
        message = BytesParser(policy=policy.default).parsebytes(
            b'Content-Type: ' + response.headers['content-type'].encode() + b'\r\n\r\n' + response.content)
        parts = list(message.iter_parts())
        assert all(part['content-type'] == 'text/plain; charset=utf-8' for part in parts)
        assert [part['content-range'] for part in parts] == ['bytes 0-0/3', 'bytes 2-2/3']
        assert [part.get_payload(decode=True) for part in parts] == [b'n', b'w']
        assert int(response.headers['content-length']) == len(response.content)
    if method == 'HEAD' and status == 206:
        assert response.headers['content-length'] == '2'
    if status == 304:
        assert 'content-type' not in response.headers
        assert 'content-length' not in response.headers
    if status == 416:
        assert 'last-modified' not in response.headers
        assert response.headers['x-content-type-options'] == 'nosniff'


def range_sync(url, root):
    with httpx.Client(trust_env=False, timeout=10, headers={
            'x-access-token': 'fixture-token', 'accept-encoding': 'identity'}) as client:
        base = client.get(url + '/files', params={'path': f'{root}/metadata'})
        assert base.status_code == 200 and base.content == b'new'
        assert base.headers['content-type'] == 'text/plain; charset=utf-8'
        assert base.headers['content-disposition'] == 'inline; filename=metadata'
        for method, name, headers, status, expected in range_requests(base.headers['last-modified']):
            response = client.request(method, url + '/files', params={'path': f'{root}/{name}'}, headers=headers)
            check_range(response, method, status, expected)


async def range_async(url, root):
    async with httpx.AsyncClient(trust_env=False, timeout=10, headers={
            'x-access-token': 'fixture-token', 'accept-encoding': 'identity'}) as client:
        base = await client.get(url + '/files', params={'path': f'{root}/metadata'})
        assert base.status_code == 200 and base.content == b'new'
        assert base.headers['content-type'] == 'text/plain; charset=utf-8'
        assert base.headers['content-disposition'] == 'inline; filename=metadata'
        for method, name, headers, status, expected in range_requests(base.headers['last-modified']):
            response = await client.request(method, url + '/files', params={'path': f'{root}/{name}'}, headers=headers)
            check_range(response, method, status, expected)


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
    gzip_download_sync(sandbox, url, root)
    range_sync(url, root)


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
    await gzip_download_async(sandbox, url, root)
    await range_async(url, root)


if __name__ == '__main__':
    settings = json.load(sys.stdin)
    with isolated_sdk_environment():
        run_sync(settings['url'], settings['root'] + '/sync')
        asyncio.run(run_async(settings['url'], settings['root'] + '/async'))
    print('Pinned sync/async SDK raw/multipart, gzip download, synthetic range/conditional downloads and gzip upload with SDK readback passed (test routes only).')
