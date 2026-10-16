"""Fixed SDK wire ordering and bounded file-observation failures, without VMs."""
import asyncio
import copy
from contextlib import ExitStack
from dataclasses import replace
from datetime import datetime, timezone
from email.parser import BytesParser
from email.policy import default
from pathlib import Path
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch
from urllib.parse import parse_qs
from wsgiref.simple_server import make_server

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import FileNotFoundException, InvalidArgumentException
from e2b.sandbox.filesystem.filesystem import EntryInfo, FileType
from e2b.sandbox.commands.command_handle import CommandResult
from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, ThreadedEnvd

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_file_compare as files
from sdk_guest_compare import compare
from sdk_observations import Calls, value
from sdk_command_input import call

DIRECTORY = '/tmp/shared-octet'
FIRST, BAD, LAST = [item['path'] for item in files.entries(DIRECTORY)]
MTIME = 1_700_000_000


def entry(path=FIRST, size=len(files.DATA), owner='ubuntu'):
    return EntryInfo(name=path.rsplit('/', 1)[-1], path=path, type=FileType.FILE,
                     size=size, mode=0o600, permissions='rw-------', owner=owner, group=owner,
                     modified_time=datetime.fromtimestamp(MTIME, timezone.utc))


class Clock:
    def __init__(self):
        self.now = 0

    def __call__(self):
        return self.now

    async def pause(self, seconds):
        self.now += seconds


class ObservationTests(unittest.IsolatedAsyncioTestCase):
    def calls(self, replies, data=files.DATA):
        self.stat = AsyncMock(side_effect=replies)
        self.read = AsyncMock(return_value=data)
        return Calls(SimpleNamespace(files=SimpleNamespace(get_info=self.stat, read=self.read)), True)

    async def observe(self, replies, *, asynchronous=False, data=files.DATA):
        calls, evidence, clock = self.calls(replies, data), {}, Clock()
        await files.settle(calls, DIRECTORY, 'ubuntu', asynchronous, evidence,
                           clock=clock, pause=clock.pause)
        return evidence

    async def test_absence_partial_size_and_owner_are_observed_without_upload_replay(self):
        evidence = await self.observe([FileNotFoundException('not yet'), entry(size=1),
            entry(owner='root'), entry(), entry(LAST)], asynchronous=False)
        self.assertEqual(evidence['status'], 'complete')
        self.assertEqual([row['method'] for row in evidence['calls']], ['files.get_info'] * 4 + ['files.read'])
        self.read.assert_awaited_once_with(FIRST, format='bytes', user='root', request_timeout=14.94)
        self.assertEqual(evidence['calls'][0]['error']['message'], 'not yet')
        timeouts = [row['kwargs']['request_timeout'] for row in evidence['calls']]
        self.assertEqual(timeouts, sorted(timeouts, reverse=True))

    async def test_parallel_uploads_can_finish_in_either_order(self):
        evidence = await self.observe([entry(size=1), entry(LAST), entry()], asynchronous=True)
        self.assertEqual([row['args'][0] for row in evidence['calls'] if row['method'] == 'files.read'],
                         [LAST, FIRST])
        self.assertEqual(evidence['expected_files'], [FIRST, LAST])

    async def test_disappearance_shrink_wrong_identity_type_size_and_bytes_fail_with_evidence(self):
        invalid = [[entry(size=2), FileNotFoundException('disappeared')],
                   [entry(size=2), entry(size=1)], [entry('/tmp/unrelated')],
                   [replace(entry(), type=FileType.DIR)], [entry(size=True)],
                   [entry(size=len(files.DATA)+1)]]
        for replies in invalid:
            with self.subTest(replies=replies):
                evidence, clock = {}, Clock()
                with self.assertRaises(AssertionError):
                    await files.settle(self.calls(replies), DIRECTORY, 'ubuntu', False, evidence,
                                       clock=clock, pause=clock.pause)
                self.assertEqual(evidence['status'], 'incomplete')
                self.assertTrue(evidence['calls'])
        evidence, clock = {}, Clock()
        with self.assertRaisesRegex(AssertionError, 'bytes differ'):
            await files.settle(self.calls([entry()], b'wrong'), DIRECTORY, 'ubuntu', False, evidence,
                               clock=clock, pause=clock.pause)
        self.assertEqual(evidence['calls'][-1]['result'], value(b'wrong'))

    async def test_deadline_after_response_count_budget_and_unexpected_error_remain_incomplete(self):
        clock, evidence = Clock(), {}
        async def late(*_args, **_kwargs):
            clock.now = files.OBSERVATION_SECONDS
            return entry()
        calls = self.calls(late)
        with self.assertRaisesRegex(AssertionError, 'deadline'):
            await files.settle(calls, DIRECTORY, 'ubuntu', False, evidence, clock=clock, pause=clock.pause)
        self.assertEqual(evidence['status'], 'incomplete')
        self.assertEqual(evidence['calls'][0]['result'], value(entry()))
        self.read.assert_not_called()
        clock, evidence = Clock(), {}
        with patch.object(files, 'OBSERVATION_ROUNDS', 2), self.assertRaisesRegex(AssertionError, 'count'):
            await files.settle(self.calls([FileNotFoundException('missing')] * 2), DIRECTORY,
                               'ubuntu', False, evidence, clock=clock, pause=clock.pause)
        self.assertEqual(len(evidence['calls']), 2)
        evidence = {}
        with self.assertRaises(InvalidArgumentException):
            await files.settle(self.calls([InvalidArgumentException('wrong error')]), DIRECTORY,
                               'ubuntu', False, evidence, clock=clock, pause=clock.pause)
        self.assertEqual(evidence['calls'][0]['status'], 'incomplete')

    async def test_allow_success_is_explicit_and_does_not_accept_other_errors(self):
        for reply, allow, failure in [(entry(), True, None),
                                      (FileNotFoundException('missing'), True, None),
                                      (entry(), False, AssertionError),
                                      (InvalidArgumentException('wrong'), True, InvalidArgumentException)]:
            calls = self.calls([reply])
            async def invoke():
                return await calls.invoke('files.get_info', FIRST, expected=FileNotFoundException,
                                          allow_success=allow)
            if failure:
                with self.assertRaises(failure):
                    await invoke()
                self.assertEqual(calls.rows[0]['status'], 'incomplete')
            else:
                self.assertIs(await invoke(), reply)
                self.assertEqual(calls.rows[0]['status'], 'complete')
            self.assertIn('result' if isinstance(reply, EntryInfo) else 'error', calls.rows[0])

    async def test_final_info_checks_actual_mtime_path_owner_kind_size_and_permissions(self):
        for changed in [entry(), replace(entry(), path=LAST), replace(entry(), owner='root'),
                        replace(entry(), type=FileType.DIR), replace(entry(), size=0),
                        replace(entry(), mode=0o644), replace(entry(), modified_time=datetime.now(timezone.utc))]:
            calls = self.calls([changed])
            async def observe():
                return await files.info(calls, FIRST, MTIME, owner='ubuntu', kind=FileType.FILE,
                                        size=len(files.DATA), mode=0o600)
            if changed == entry():
                self.assertEqual(await observe(), changed)
            else:
                with self.assertRaises(AssertionError):
                    await observe()
            self.assertEqual(calls.rows[0]['result'], value(changed))

    async def test_private_directory_case_stops_on_bad_identity_or_original_owner(self):
        inputs = dict(directory='/tmp/shared', mtime=MTIME, users=['root', 'ubuntu'],
                      unknown_user='sp_missing_fixed')
        secret = '/tmp/shared/files-context/private/secret'
        for identity in ('root\n', 'ubuntu\n'):
            outputs = iter(['', identity, ''])
            async def command(_text, **kwargs):
                output = next(outputs)
                kwargs['on_stdout'](output)
                return CommandResult(stdout=output, stderr='', exit_code=0, error=None)
            commands = SimpleNamespace(run=AsyncMock(side_effect=command))
            filesystem = SimpleNamespace(read=AsyncMock(return_value='original'),
                get_info=AsyncMock(return_value=entry(secret, size=8, owner='ubuntu')),
                write=AsyncMock())
            calls = Calls(SimpleNamespace(commands=commands, files=filesystem), True)
            with self.assertRaises(AssertionError):
                await files.file_cases(calls, inputs)
            filesystem.write.assert_not_called()
            self.assertEqual(commands.run.await_args_list[1].kwargs['user'], 'ubuntu')
            self.assertIn('if test -r ' + secret, commands.run.await_args_list[1].args[0])
            if identity == 'root\n':
                filesystem.read.assert_not_called()
                self.assertEqual(calls.rows[-1]['result']['stdout'], identity)
            else:
                filesystem.read.assert_awaited_once_with(secret, user='ubuntu', request_timeout=15)
                self.assertIn('find /tmp/shared/files-context/private -exec touch -h -d @1700000000',
                              commands.run.await_args_list[2].args[0])
                self.assertEqual(calls.rows[-1]['result']['owner'], 'ubuntu')

    async def test_batch_unknown_reply_or_bad_followup_keeps_one_original_attempt(self):
        inputs = dict(directory='/tmp/shared', mtime=MTIME, users=['root', 'ubuntu'])
        directory = inputs['directory'] + '/octet-root'
        for error in (TimeoutError('upload outcome unknown'), InvalidArgumentException('directory')):
            filesystem = SimpleNamespace(make_dir=AsyncMock(return_value=True),
                write_files=AsyncMock(side_effect=error),
                get_info=AsyncMock(return_value=entry(directory + '/first', size=len(files.DATA)+1)))
            rows = []
            expected = TimeoutError if isinstance(error, TimeoutError) else AssertionError
            with self.assertRaises(expected):
                await files.sdk_file_cases(SimpleNamespace(files=filesystem), inputs, True, rows)
            filesystem.write_files.assert_awaited_once_with(files.entries(directory), user='root',
                                                             use_octet_stream=True, request_timeout=15)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]['status'], 'incomplete')
            self.assertEqual(rows[0]['calls'][-1]['error']['message'], str(error))
            if expected is TimeoutError:
                filesystem.get_info.assert_not_called()
                self.assertEqual(rows[0]['settlement'], {})
            else:
                self.assertEqual(rows[0]['settlement']['status'], 'incomplete')
                self.assertEqual(len(rows[0]['settlement']['calls']), 1)

    async def test_only_validated_transient_sampling_is_excluded_from_exact_comparison(self):
        first = await self.observe([entry()])
        delayed = await self.observe([FileNotFoundException('not yet'), entry(size=1), entry()])
        def batch(settlement):
            return dict(method='files.octet_batch', args=['ubuntu', DIRECTORY], asynchronous=False,
                        status='complete', settlement=settlement, calls=[
                dict(method='files.write_files', args=value([files.entries(DIRECTORY)]), kwargs={},
                     error={'class': 'InvalidArgumentException', 'message': 'exact directory'}, status='complete'),
                dict(method='files.get_info', args=[FIRST], kwargs={}, result=value(entry()), status='complete'),
                dict(method='files.read', args=[FIRST], kwargs={}, result=value(files.DATA), status='complete')])
        original, other = batch(first), batch(delayed)
        saved = copy.deepcopy(other)
        compare([original], [other], 'valid observations')
        self.assertEqual(other, saved)
        mutations = [lambda r: r['calls'][0]['args'][0].reverse(),
            lambda r: r['calls'][0]['error'].__setitem__('message', 'changed'),
            lambda r: r['calls'][1]['result'].__setitem__('mode', 0o644),
            lambda r: r['calls'][1]['result'].__setitem__('modified_time', {'datetime': 'changed'}),
            lambda r: r['calls'][2].__setitem__('result', value(b'wrong')),
            lambda r: r['calls'].reverse(), lambda r: r.__setitem__('asynchronous', True),
            lambda r: r['settlement'].__setitem__('status', 'incomplete'),
            lambda r: r['settlement']['calls'][-1].__setitem__('result', value(b'wrong')),
            lambda r: r['settlement']['calls'][-2]['result'].__setitem__('owner', 'root'),
            lambda r: r['settlement']['calls'][0]['error'].__setitem__('class', 'OtherError'),
            lambda r: r['settlement']['calls'][0].__setitem__('method', 'files.write'),
            lambda r: r['settlement']['calls'][0]['kwargs'].__setitem__('request_timeout', 0),
            lambda r: r['settlement']['calls'].pop()]
        for mutation in mutations:
            changed = copy.deepcopy(other)
            mutation(changed)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                compare([original], [changed], 'changed evidence')


class GatedEnvd(SyntheticEnvd):
    """Preset HTTP replies with independent barriers; no filesystem emulation."""
    def __init__(self):
        super().__init__()
        self.parallel = False
        self.started = {path: threading.Event() for path in (FIRST, LAST)}
        self.release = threading.Event()
        self.file_status_by_path[BAD] = 400

    def __call__(self, environ, start_response):
        if self.parallel and environ['PATH_INFO'] == '/files' and environ['REQUEST_METHOD'] == 'POST':
            path = parse_qs(environ.get('QUERY_STRING', '')).get('path', [''])[0]
            if path in self.started:
                self.started[path].set()
                if not self.release.wait(10):
                    raise AssertionError('test did not release pending upload')
            elif path == BAD:
                if not all(event.wait(10) for event in self.started.values()):
                    raise AssertionError('original async SDK did not start all uploads')
        return super().__call__(environ, start_response)


class OriginalBatchTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.app = GatedEnvd()
        self.server = make_server('127.0.0.1', 0, self.app,
                                 server_class=ThreadedEnvd, handler_class=QuietHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.app.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5)
        self.assertFalse(self.thread.is_alive())

    async def connect(self, cls):
        url = f'http://127.0.0.1:{self.server.server_port}'
        return await call(cls.connect, 'probe-sandbox', api_url=url, sandbox_url=url,
                          api_key='synthetic-only', domain='example.invalid', retries=0, request_timeout=15)

    async def test_original_sync_octet_stops_after_error_without_sending_last(self):
        sandbox = await self.connect(Sandbox)
        calls = Calls(sandbox, False)
        await calls.invoke('files.write_files', files.entries(DIRECTORY), user='ubuntu',
                           use_octet_stream=True, request_timeout=15, expected=InvalidArgumentException)
        requests = [r for r in self.app.records if r['path'] == '/files']
        self.assertEqual([r['query']['path'][0] for r in requests], [FIRST, BAD])
        for request in requests:
            self.assertEqual(request['body'], files.DATA)
            self.assertEqual(request['query']['username'], ['ubuntu'])
            self.assertEqual(request['content_type'], 'application/octet-stream')
        self.assertEqual(calls.rows[0]['error']['class'], 'e2b.exceptions.InvalidArgumentException')

    async def test_original_async_error_returns_while_other_uploads_are_still_pending(self):
        sandbox = await self.connect(AsyncSandbox)
        self.app.parallel = True
        original_post = sandbox.files._envd_api.post
        completed = {path: asyncio.Event() for path in (FIRST, LAST)}
        deadlines = []
        async def observed_post(*args, **kwargs):
            deadlines.append(kwargs['timeout'])
            try:
                return await original_post(*args, **kwargs)
            finally:
                path = kwargs['params']['path']
                if path in completed:
                    completed[path].set()
        # Observe completion of the original HTTP method; no synthetic SDK return.
        with patch.object(sandbox.files._envd_api, 'post', observed_post):
            try:
                calls = Calls(sandbox, True)
                await calls.invoke('files.write_files', files.entries(DIRECTORY), user='ubuntu',
                                   use_octet_stream=True, request_timeout=15, expected=InvalidArgumentException)
                self.assertTrue(all(event.is_set() for event in self.app.started.values()))
                self.assertFalse(any(event.is_set() for event in completed.values()))
                self.assertEqual(deadlines, [15, 15, 15])
            finally:
                self.app.release.set()
                await asyncio.wait_for(asyncio.gather(*(event.wait() for event in completed.values())), 10)
        requests = [r for r in self.app.records if r['path'] == '/files']
        self.assertCountEqual([r['query']['path'][0] for r in requests], [FIRST, BAD, LAST])
        self.assertTrue(all(r['body'] == files.DATA for r in requests))

    async def test_original_multipart_retains_input_order_and_both_modes_parse_same_error(self):
        observed = []
        self.app.file_status = 400
        for cls in (Sandbox, AsyncSandbox):
            calls = Calls(await self.connect(cls), cls is AsyncSandbox)
            await calls.invoke('files.write_files', files.entries(DIRECTORY), user='ubuntu',
                               use_octet_stream=False, request_timeout=15, expected=InvalidArgumentException)
            observed.append(calls.rows)
        compare(*observed, 'multipart fixed SDK')
        requests = [r for r in self.app.records if r['path'] == '/files']
        self.assertEqual(len(requests), 2)
        for request in requests:
            message = BytesParser(policy=default).parsebytes(
                b'Content-Type: ' + request['content_type'].encode() + b'\r\n\r\n' + request['body'])
            parts = list(message.iter_parts())
            self.assertEqual([part.get_filename() for part in parts], [FIRST, BAD, LAST])
            self.assertTrue(all(part.get_payload(decode=True) == files.DATA for part in parts))
            self.assertEqual(request['query'], {'username': ['ubuntu']})


if __name__ == '__main__':
    unittest.main()
