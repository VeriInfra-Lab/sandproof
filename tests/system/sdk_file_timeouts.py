"""Original SDK file request/idle timeout values against a real fixed base.

Requires explicit streaming budgets and envd supporting octet-stream uploads.
The large HTTP timeout is a valid control above Connect's distinct header limit.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import hashlib
import importlib.metadata
import io
import json
import os
import uuid

import httpx
from e2b import AsyncSandbox, Sandbox

from sdk_command_input import call, destroy_probes
from sdk_data_contract import guest_arguments, guest_candidate, observe_candidate
from sdk_server_smoke import options


VALUES = [('omitted', None), ('zero', 0), ('negative', -1), ('large', 10_000_000)]
PAYLOAD = (b'file-timeout-integrity\n' * 3000)
DIGEST = hashlib.sha256(PAYLOAD).hexdigest()


def arguments(field, case, value):
    return {} if case == 'omitted' else {field: value}


def verify_deadline(error):
    # The pinned pyqwest HTTP adapter maps its operation deadline to ReadTimeout.
    # A transport EOF, SDK SandboxException, or arbitrary exception is not this.
    assert type(error) is httpx.ReadTimeout
    assert isinstance(error.__cause__, TimeoutError)
    # SDK reader cleanup can raise a second exception while handling an
    # unrelated read failure/cancellation. That is not an accepted deadline.
    assert error.__context__ is None or error.__context__ is error.__cause__, 'deadline hides another failure'


async def read_content(sandbox, path, format, *, observation=None, **kwargs):
    # This probe reads only its fixed payload or an observed prefix of an
    # unknown upload. Bound its own accumulation; this is not a product limit.
    data, digest = bytearray(), hashlib.sha256()
    observation = {} if observation is None else observation
    observation.update(public_bytes=0, public_sha256=digest.hexdigest(), public_eof=False)
    def append(chunk):
        assert isinstance(chunk, (bytes, bytearray)), 'non-byte file chunk'
        end = len(data) + len(chunk)
        assert end <= len(PAYLOAD), 'file observation exceeds fixed payload'
        assert chunk == PAYLOAD[len(data):end], 'file observation has a corrupt prefix'
        data.extend(chunk)
        digest.update(chunk)
        observation.update(public_bytes=len(data), public_sha256=digest.hexdigest())

    result = await call(sandbox.files.read, path, format=format, **kwargs)
    if format == 'text':
        append(result.encode())
    elif format == 'bytes':
        append(result)
    elif hasattr(result, '__aiter__'):
        primary = None
        try:
            async for chunk in result:
                append(chunk)
        except BaseException as error:
            primary = error
        try:
            await result.aclose()
        except BaseException as cleanup:
            if primary is not None:
                raise BaseExceptionGroup('file read and close failed', [primary, cleanup])
            raise
        if primary is not None:
            raise primary
    else:
        def consume():
            primary = None
            try:
                for chunk in result:
                    append(chunk)
            except BaseException as error:
                primary = error
            try:
                result.close()
            except BaseException as cleanup:
                if primary is not None:
                    raise BaseExceptionGroup('file read and close failed', [primary, cleanup])
                raise
            if primary is not None:
                raise primary
        await asyncio.to_thread(consume)
    observation['public_eof'] = True
    return bytes(data)


async def read_case(sandbox, path, asynchronous, format, field, case, value):
    expected_deadline = case == 'negative' and (field == 'request_timeout' or asynchronous)
    observation = {}
    try:
        content = await read_content(sandbox, path, format, observation=observation,
                                     **arguments(field, case, value))
    except Exception as error:
        assert expected_deadline
        verify_deadline(error)
    else:
        assert not expected_deadline, 'negative deadline returned success'
        assert content == PAYLOAD and hashlib.sha256(content).hexdigest() == DIGEST
    control = await read_content(sandbox, path, 'bytes')
    assert control == PAYLOAD
    return dict(operation='read', format=format, parameter=field, case=case,
                outcome='deadline' if expected_deadline else 'completed', read_observation=observation)


def make_source(kind):
    if kind == 'bytes':
        return PAYLOAD
    if kind == 'text':
        return PAYLOAD.decode()
    if kind == 'binary_io':
        return io.BytesIO(PAYLOAD)
    assert kind == 'text_io'
    return io.StringIO(PAYLOAD.decode())


async def write_case(sandbox, directory, batch, octet, kind, case, value):
    paths = [f'{directory}/{uuid.uuid4().hex}' for _ in range(2 if batch else 1)]
    sources = [make_source(kind) for _ in paths]
    streamed = kind == 'binary_io' or (kind == 'text_io' and octet)
    expected_deadline = case == 'negative' and not streamed
    try:
        kwargs = dict(use_octet_stream=octet, **arguments('request_timeout', case, value))
        try:
            if batch:
                result = await call(sandbox.files.write_files,
                                    [dict(path=p, data=s) for p, s in zip(paths, sources)], **kwargs)
            else:
                result = [await call(sandbox.files.write, paths[0], sources[0], **kwargs)]
        except Exception as error:
            assert expected_deadline
            verify_deadline(error)
        else:
            assert not expected_deadline, 'negative buffered upload deadline returned success'
            assert len(result) == len(paths)
            assert [entry.path for entry in result] == paths
        observed, digests = [], []
        for path in paths:
            exists = await call(sandbox.files.exists, path)
            if not expected_deadline:
                assert exists
            if exists:
                content = await read_content(sandbox, path, 'bytes')
                if expected_deadline:
                    # A deadline is not rollback: a partial or complete write
                    # can already have occurred. Observe it without replaying.
                    assert len(content) <= len(PAYLOAD) and PAYLOAD.startswith(content)
                else:
                    assert content == PAYLOAD and hashlib.sha256(content).hexdigest() == DIGEST
                observed.append(len(content))
                digests.append(hashlib.sha256(content).hexdigest())
            else:
                observed.append(None)
                digests.append(None)
    finally:
        for source in sources:
            if isinstance(source, io.IOBase):
                source.close()
    control = f'{directory}/{uuid.uuid4().hex}-control'
    await call(sandbox.files.write, control, PAYLOAD)
    control_content = await read_content(sandbox, control, 'bytes')
    assert control_content == PAYLOAD
    return dict(operation='write_files' if batch else 'write', encoding='octet' if octet else 'multipart',
                source=kind, parameter='request_timeout', case=case,
                outcome='deadline' if expected_deadline else 'completed',
                observed_lengths=observed, observed_sha256=digests)


async def exercise(args):
    assert importlib.metadata.version('e2b') == '2.51.0'
    template, candidate = guest_candidate(args)
    owner = options(os.environ['SANDPROOF_TEST_API_KEY'], args.api_url, args.sandbox_url)
    results = []
    for cls in (Sandbox, AsyncSandbox):
        sandbox = await call(cls.create, template=template, timeout=300,
                             allow_internet_access=False, **owner)
        try:
            before = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            directory = '/tmp/file-timeouts-' + uuid.uuid4().hex
            path = directory + '/read-control'
            await call(sandbox.files.write, path, PAYLOAD)
            rows = []
            for format in ('text', 'bytes', 'stream'):
                for case, value in VALUES:
                    rows.append(await read_case(sandbox, path, cls is AsyncSandbox, format,
                                                'request_timeout', case, value))
            for case, value in VALUES:
                rows.append(await read_case(sandbox, path, cls is AsyncSandbox, 'stream',
                                            'stream_idle_timeout', case, value))
            for batch in (False, True):
                for octet in (False, True):
                    for kind in ('bytes', 'text', 'binary_io', 'text_io'):
                        for case, value in VALUES:
                            rows.append(await write_case(sandbox, directory, batch, octet,
                                                         kind, case, value))
            assert (await call(sandbox.commands.run, 'printf healthy', timeout=15)).stdout == 'healthy'
            after = await observe_candidate(sandbox, candidate, asynchronous=cls is AsyncSandbox)
            assert before == after, 'Rust guest identity changed during file timeout cases'
            item = dict(sdk=cls.__name__, cases=rows)
            if candidate is not None:
                item.update(sandbox_id=sandbox.sandbox_id, rust_guest=before)
            results.append(item)
        finally:
            await destroy_probes([sandbox])
    report = dict(sdk_version='2.51.0', payload_bytes=len(PAYLOAD), sha256=DIGEST,
                  results=results, scope='file HTTP request and stream idle timeout values; '
                  'explicit multipart/octet modes; not unary RPC or lifetime deadlines')
    if getattr(args, 'template', None) is not None:
        report['template'] = template
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--sandbox-url', required=True)
    guest_arguments(parser)
    asyncio.run(exercise(parser.parse_args()))
