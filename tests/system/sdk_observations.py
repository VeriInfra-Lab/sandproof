"""Bounded original SDK call evidence shared by paired guest probes."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import asyncio
import base64
from dataclasses import asdict, is_dataclass
from datetime import datetime
from enum import Enum
from functools import partial
from io import BytesIO
from sdk_command_input import call

CONTENT_LIMIT = 128 * 1024


def value(item):
    """Lossless JSON representation for the fixed SDK's selected result types."""
    if isinstance(item, BytesIO):
        assert item.tell() == 0, 'each upload needs a fresh stream'
        return value(item.getvalue())
    if isinstance(item, (bytes, bytearray)):
        assert len(item) <= CONTENT_LIMIT
        return {'base64': base64.b64encode(item).decode()}
    if isinstance(item, datetime):
        return {'datetime': item.isoformat()}
    if isinstance(item, Enum):
        return item.value
    if is_dataclass(item):
        return value(asdict(item))
    if isinstance(item, dict):
        return {key: value(part) for key, part in item.items()}
    if isinstance(item, (list, tuple)):
        return [value(part) for part in item]
    assert item is None or type(item) in (bool, int, float, str), type(item)
    if isinstance(item, str):
        assert len(item.encode()) <= CONTENT_LIMIT
    return item


class Capture:
    def __init__(self):
        self.parts, self.size = [], 0

    def __call__(self, part):
        assert type(part) is str
        self.size += len(part.encode())
        assert self.size <= CONTENT_LIMIT, 'command callback budget'
        self.parts.append(part)


class Calls:
    def __init__(self, sandbox, asynchronous):
        self.sandbox, self.asynchronous = sandbox, asynchronous
        self.rows = []

    async def invoke(self, method, *args, expected=None, **kwargs):
        target = self.sandbox
        for segment in method.split('.'):
            target = getattr(target, segment)
        callbacks = None
        if method == 'commands.run':
            callbacks = {'stdout': Capture(), 'stderr': Capture()}
            target = partial(target, on_stdout=callbacks['stdout'], on_stderr=callbacks['stderr'])
        def validate(result):
            if callbacks is not None:
                for channel, capture in callbacks.items():
                    assert ''.join(capture.parts) == getattr(result, channel, ''), channel
        evidence = {} if callbacks is None else {
            'callbacks': {channel: capture.parts for channel, capture in callbacks.items()}}
        return await self.operation(method, target, *args, expected=expected,
                                    evidence=evidence, validate=validate, **kwargs)

    async def operation(self, method, target, *args, expected=None, serialize=value,
                        evidence=None, validate=None, **kwargs):
        row = {'method': method, 'args': value(args), 'kwargs': value(kwargs),
               'status': 'incomplete', **(evidence or {})}
        self.rows.append(row)
        try:
            result = await call(target, *args, **kwargs)
        except Exception as error:
            result = error
            row['error'] = {'class': type(error).__module__ + '.' + type(error).__qualname__,
                            'message': value(str(error))}
            if is_dataclass(error):
                row['error']['fields'] = value(error)
            if expected is None or type(error) is not expected:
                raise
        else:
            row['result'] = serialize(result)
            assert expected is None, (method, 'expected failure returned success')
        if validate is not None:
            validate(result)
        row['status'] = 'complete'
        return result

    async def stream(self, path, user):
        kwargs = dict(format='stream', user=user, gzip=True, request_timeout=15,
                      stream_idle_timeout=15)
        row = {'method': 'files.read', 'args': [path], 'kwargs': value(kwargs), 'status': 'incomplete'}
        self.rows.append(row)
        data, chunks = bytearray(), []
        def append(chunk):
            assert len(data) + len(chunk) <= CONTENT_LIMIT, 'file stream budget'
            data.extend(chunk)
            chunks.append(len(chunk))
        if self.asynchronous:
            async with await self.sandbox.files.read(path, **kwargs) as reader:
                async for chunk in reader:
                    append(chunk)
        else:
            def drain():
                with self.sandbox.files.read(path, **kwargs) as reader:
                    for chunk in reader:
                        append(chunk)
            await asyncio.to_thread(drain)
        row.update(result=value(data), transport_chunk_bytes=chunks, status='complete')
        return data
