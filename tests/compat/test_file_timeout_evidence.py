"""Fixed SDK file methods with prescribed HTTP/RPC replies, not an envd."""
import asyncio
from collections import deque
from contextlib import ExitStack
from email import policy
from email.parser import BytesParser
import hashlib
import io
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

import httpx
from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.sandbox_sync.filesystem import filesystem as sync_fs
from e2b.sandbox_async.filesystem import filesystem as async_fs
from packaging.version import Version
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_file_timeouts as probe


def deadline():
    error = httpx.ReadTimeout('prescribed transfer deadline')
    error.__cause__ = TimeoutError('prescribed clock expiry')
    return error


class Body(httpx.SyncByteStream, httpx.AsyncByteStream):
    def __init__(self, events, close_error=None):
        self.events, self.reads, self.closes = events, 0, 0
        self.close_error = close_error

    def __iter__(self):
        for item in self.events:
            self.reads += 1
            if isinstance(item, BaseException):
                raise item
            yield item

    async def __aiter__(self):
        for item in self:
            yield item

    def close(self):
        self.closes += 1
        if self.close_error is not None:
            raise self.close_error

    async def aclose(self):
        self.close()


class Tape:
    """Only scheduled replies; it never applies writes or models a filesystem."""
    def __init__(self, asynchronous, *, gets=(), post_fault=None, reverse=False, absent=(), close_error=None):
        self.asynchronous = asynchronous
        self.gets, self.records, self.bodies, self.stats = deque(gets), [], [], []
        self.post_fault, self.reverse, self.absent = post_fault, reverse, set(absent)
        self.close_error = close_error
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(stat=wrap(side_effect=self.stat))
        if asynchronous:
            self.client = httpx.AsyncClient(base_url='http://fixture.invalid', transport=httpx.MockTransport(self.async_http))
            module = async_fs
        else:
            self.client = httpx.Client(base_url='http://fixture.invalid', transport=httpx.MockTransport(self.sync_http))
            module = sync_fs
        config = ConnectionConfig(api_key='synthetic-only', retries=0, request_timeout=19)
        with patch.object(module, 'create_rpc_client', return_value=rpc), patch.object(module, 'get_envd_api', return_value=self.client):
            self.sandbox = SimpleNamespace(files=module.Filesystem('http://fixture.invalid', Version('0.9.0'), config, self.client))

    def stat(self, request, **options):
        self.stats.append((request.path, options))
        if request.path in self.absent:
            raise ConnectError(Code.NOT_FOUND, 'prescribed absence')
        return None  # The original exists() only requires a successful reply.

    def sync_http(self, request):
        request.read()
        return self.http(request)

    async def async_http(self, request):
        await request.aread()
        return self.http(request)

    def http(self, request):
        self.records.append(request)
        if request.method == 'GET':
            script = self.gets.popleft()
            if isinstance(script, BaseException):
                raise script
            body = Body(script, self.close_error if not self.bodies else None)
            self.bodies.append(body)
            return httpx.Response(200, stream=body, headers={'Content-Type': 'text/plain; charset=utf-8'})
        assert request.method == 'POST'
        if self.post_fault is not None and not request.url.params.get('path', '').endswith('-control'):
            raise self.post_fault
        if request.headers['content-type'].startswith('multipart/'):
            message = BytesParser(policy=policy.default).parsebytes(
                b'Content-Type: ' + request.headers['content-type'].encode() + b'\r\n\r\n' + request.content)
            parts = list(message.iter_parts())
            paths = [part.get_filename() for part in parts]
            assert all(part.get_payload(decode=True) == probe.PAYLOAD for part in parts)
        else:
            paths = [request.url.params['path']]
            assert request.content == probe.PAYLOAD
        if self.reverse:
            paths.reverse()
        return httpx.Response(200, json=[dict(name=p.rsplit('/', 1)[1], path=p, type='file') for p in paths])

    async def close(self):
        if self.asynchronous:
            await self.client.aclose()
        else:
            self.client.close()


class FileEvidenceTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.tapes = []

    async def asyncTearDown(self):
        for tape in self.tapes:
            await tape.close()

    def tape(self, asynchronous, **kwargs):
        tape = Tape(asynchronous, **kwargs)
        self.tapes.append(tape)
        return tape

    async def test_stream_overflow_and_corruption_stop_before_tail_and_close(self):
        for asynchronous in (False, True):
            for prefix in (probe.PAYLOAD + b'x', b'corrupt prefix'):
                tape = self.tape(asynchronous, gets=[[prefix, RuntimeError('tail must not be consumed')]])
                with self.subTest(asynchronous=asynchronous, prefix=len(prefix)), self.assertRaises(AssertionError):
                    await probe.read_content(tape.sandbox, '/payload', 'stream')
                self.assertEqual(tape.bodies[0].reads, 1)
                self.assertEqual(tape.bodies[0].closes, 1)

    async def test_partial_deadline_records_only_public_prefix_without_replay(self):
        prefix = probe.PAYLOAD[:77]
        for asynchronous in (False, True):
            tape = self.tape(asynchronous, gets=[[prefix[:20], prefix[20:], deadline()], [probe.PAYLOAD]])
            row = await probe.read_case(tape.sandbox, '/payload', asynchronous, 'stream', 'request_timeout', 'negative', -1)
            self.assertEqual(row['outcome'], 'deadline')
            self.assertEqual(row['read_observation'], dict(public_bytes=77,
                public_sha256=hashlib.sha256(prefix).hexdigest(), public_eof=False))
            self.assertEqual(len(tape.records), 2)
            self.assertEqual([r.extensions['timeout']['read'] for r in tape.records], [-1, 19])
            self.assertTrue(all(body.closes == 1 for body in tape.bodies))

    async def test_corrupt_prefix_cannot_be_accepted_as_expected_deadline(self):
        for asynchronous in (False, True):
            tape = self.tape(asynchronous, gets=[[b'corrupt', deadline()], [probe.PAYLOAD]])
            with self.assertRaises(AssertionError):
                await probe.read_case(tape.sandbox, '/payload', asynchronous, 'stream', 'request_timeout', 'negative', -1)
            self.assertEqual(len(tape.records), 1)
            self.assertEqual(tape.bodies[0].reads, 1)
            self.assertEqual(tape.bodies[0].closes, 1)

    async def test_integrity_and_close_failures_are_retained_together(self):
        for asynchronous in (False, True):
            close_error = deadline()
            tape = self.tape(asynchronous, gets=[[b'corrupt', RuntimeError('unread tail')]], close_error=close_error)
            with self.assertRaises(BaseExceptionGroup) as caught:
                await probe.read_content(tape.sandbox, '/payload', 'stream')
            self.assertIsInstance(caught.exception.exceptions[0], AssertionError)
            self.assertIs(caught.exception.exceptions[1], close_error)
            self.assertEqual(tape.bodies[0].reads, 1)
            self.assertEqual(tape.bodies[0].closes, 1)

    async def test_sdk_close_timeout_cannot_hide_transport_failure_or_cancellation(self):
        for asynchronous in (False, True):
            for primary in (httpx.ReadError('truncated transfer'), asyncio.CancelledError()):
                tape = self.tape(asynchronous, gets=[[probe.PAYLOAD[:77], primary], [probe.PAYLOAD]], close_error=deadline())
                with self.subTest(asynchronous=asynchronous, primary=type(primary).__name__), self.assertRaises(AssertionError):
                    await probe.read_case(tape.sandbox, '/payload', asynchronous, 'stream', 'request_timeout', 'negative', -1)
                self.assertEqual(len(tape.records), 1)
                self.assertEqual(tape.bodies[0].closes, 1)

    async def test_truncation_wrong_timeout_cause_and_cancellation_are_not_accepted(self):
        for asynchronous in (False, True):
            for format in ('text', 'bytes', 'stream'):
                tape = self.tape(asynchronous, gets=[[probe.PAYLOAD[:-1]]])
                with self.assertRaises(AssertionError):
                    await probe.read_case(tape.sandbox, '/payload', asynchronous, format, 'request_timeout', 'omitted', None)
                self.assertEqual(len(tape.records), 1)
            no_cause, wrong_cause = httpx.ReadTimeout('missing cause'), httpx.ReadTimeout('wrong cause')
            wrong_cause.__cause__ = OSError('not an operation deadline')
            for error in (no_cause, wrong_cause, httpx.ReadError('transport failure')):
                tape = self.tape(asynchronous, gets=[error])
                with self.assertRaises(AssertionError):
                    await probe.read_case(tape.sandbox, '/payload', asynchronous, 'stream', 'request_timeout', 'negative', -1)
                self.assertEqual(len(tape.records), 1)
            tape = self.tape(asynchronous, gets=[[probe.PAYLOAD[:77], asyncio.CancelledError()]])
            with self.assertRaises(asyncio.CancelledError):
                await probe.read_content(tape.sandbox, '/payload', 'stream')
            self.assertEqual(tape.bodies[0].closes, 1)

    async def test_all_read_parameter_vectors_use_original_sdk_and_record_complete_or_unknown(self):
        for asynchronous in (False, True):
            for format, field in [(kind, 'request_timeout') for kind in ('text', 'bytes', 'stream')] + [('stream', 'stream_idle_timeout')]:
                for case, value in probe.VALUES:
                    negative = case == 'negative' and (field == 'request_timeout' or asynchronous)
                    idle_before_send = asynchronous and field == 'stream_idle_timeout' and case == 'negative'
                    scripts = [deadline()] if negative else [[probe.PAYLOAD[:31], probe.PAYLOAD[31:]]]
                    tape = self.tape(asynchronous, gets=([] if idle_before_send else scripts) + [[probe.PAYLOAD]])
                    row = await probe.read_case(tape.sandbox, '/payload', asynchronous, format, field, case, value)
                    self.assertEqual(row['outcome'], 'deadline' if negative else 'completed')
                    observation = row['read_observation']
                    self.assertEqual(observation, dict(public_bytes=0 if negative else len(probe.PAYLOAD),
                        public_sha256=hashlib.sha256(b'').hexdigest() if negative else probe.DIGEST,
                        public_eof=not negative))
                    self.assertFalse(tape.gets)
                    if not idle_before_send:
                        expected = -1 if case == 'negative' and field == 'request_timeout' else (
                            10_000_000 if case == 'large' and field == 'request_timeout' else
                            None if format == 'stream' or case == 'zero' else 19)
                        self.assertEqual(tape.records[0].extensions['timeout']['read'], expected)

    async def test_reordered_batch_reply_is_rejected_before_observations_or_control_write(self):
        for asynchronous in (False, True):
            tape = self.tape(asynchronous, reverse=True, gets=[[probe.PAYLOAD]] * 3)
            with self.assertRaises(AssertionError):
                await probe.write_case(tape.sandbox, '/batch', True, False, 'bytes', 'omitted', None)
            self.assertEqual(len(tape.records), 1)
            self.assertFalse(tape.stats)

    async def test_all_upload_vectors_keep_original_deadlines_and_close_every_source(self):
        for asynchronous in (False, True):
            for batch in (False, True):
                for octet in (False, True):
                    for kind in ('bytes', 'text', 'binary_io', 'text_io'):
                        for case, value in probe.VALUES:
                            streamed = kind == 'binary_io' or (kind == 'text_io' and octet)
                            negative = case == 'negative' and not streamed
                            count = 2 if batch else 1
                            tape = self.tape(asynchronous, gets=[[probe.PAYLOAD]] * (count + 1), post_fault=deadline() if negative else None)
                            sources = []
                            original = probe.make_source
                            def source(kind):
                                item = original(kind)
                                sources.append(item)
                                return item
                            with patch.object(probe, 'make_source', source):
                                row = await probe.write_case(tape.sandbox, '/batch', batch, octet, kind, case, value)
                            self.assertEqual(row['outcome'], 'deadline' if negative else 'completed')
                            self.assertEqual(row['observed_lengths'], [len(probe.PAYLOAD)] * count)
                            self.assertEqual(row['observed_sha256'], [probe.DIGEST] * count)
                            self.assertEqual(len(tape.stats), count)
                            self.assertTrue(all(item.closed for item in sources if isinstance(item, io.IOBase)))
                            uploads = [r for r in tape.records if r.method == 'POST' and not r.url.params.get('path', '').endswith('-control')]
                            self.assertEqual(len(uploads), count if octet and (asynchronous or not negative) else 1)
                            expected = None if streamed or case == 'zero' else 19 if case == 'omitted' else value
                            self.assertTrue(all(r.extensions['timeout']['read'] == expected for r in uploads))
                            self.assertFalse(tape.gets)

    async def test_unknown_upload_snapshots_keep_absence_empty_and_partial_distinct(self):
        for asynchronous in (False, True):
            for content in (None, b'', probe.PAYLOAD[:77]):
                tape = self.tape(asynchronous, post_fault=deadline(), absent=['/batch/fixed'] if content is None else [],
                                 gets=([] if content is None else [[content]]) + [[probe.PAYLOAD]])
                with patch.object(probe.uuid, 'uuid4', return_value=SimpleNamespace(hex='fixed')):
                    row = await probe.write_case(tape.sandbox, '/batch', False, False, 'bytes', 'negative', -1)
                self.assertEqual(row['observed_lengths'], [None if content is None else len(content)])
                self.assertEqual(row['observed_sha256'], [None if content is None else hashlib.sha256(content).hexdigest()])
                self.assertEqual(len([r for r in tape.records if r.method == 'POST']), 2)
