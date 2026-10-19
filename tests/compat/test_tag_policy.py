"""Original fixed SDK handles with scheduled tag outcomes; not envd execution."""
import asyncio
from collections import deque
from contextlib import ExitStack
import hashlib
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.connection_config import ConnectionConfig
from e2b.envd.process import process_pb
from e2b.sandbox.commands.command_handle import CommandResult
from e2b.sandbox_sync.commands import command as sync_command
from e2b.sandbox_async.commands import command as async_command
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_tag_policy as probe
from sdk_observations import Calls

PARENT = '/tmp/policy/shared'


class Transcript:
    def __init__(self, asynchronous, kind, user, outcomes, fault):
        self.asynchronous, self.kind, self.user, self.fault = asynchronous, kind, user, fault
        self.specs = probe.definitions(PARENT + '/tag-policy', user)
        self.pids = dict(first=4321, other=4322, duplicate=4323)
        self.attempts, self.calls, self.closed, self.signals = [], [], [], []
        self.done = {name: asyncio.Event() for name in probe.NAMES}
        self.outcomes = deque(outcomes)
        self.stream_id = 0
        self.rpc_failure = None
        live = list(probe.NAMES if kind == 'go' else probe.NAMES[:2])
        self.lists = deque([[], ['first', 'other'], live,
                            *[live[i:] for i in range(1, len(live) + 1)], []])

    def start(self, request, **kwargs):
        name = probe.NAMES[len(self.attempts)]
        self.attempts.append(name)
        self.calls.append(('start', request, kwargs))
        assert request.has_field('tag') and request.tag == self.specs[name]['metadata']['tag']
        if self.fault == 'unknown-start' or self.fault == 'unknown-duplicate' and name == 'duplicate':
            raise TimeoutError('Start outcome unknown')
        if name == 'duplicate' and ((self.kind == 'rust' and self.fault != 'rust-executed')
                                    or self.fault == 'go-rejected'):
            self.rpc_failure = ConnectError(Code.ALREADY_EXISTS,
                f'process with tag {request.tag} already exists', self.details('duplicate-details'))
            raise self.rpc_failure
        return self.events(process_pb.StartResponse, name, 'start')

    def connect(self, request, **kwargs):
        self.calls.append(('connect', request, kwargs))
        assert request.process.selector.field == 'tag'
        name = self.outcomes.popleft()
        if name is None or self.fault in ('wrong-error', 'details'):
            code = Code.UNAVAILABLE if self.fault == 'wrong-error' else Code.NOT_FOUND
            self.rpc_failure = ConnectError(code, f'process with tag {request.process.selector.value} not found',
                                            self.details('details'))
            raise self.rpc_failure
        return self.events(process_pb.ConnectResponse, name, 'connect')

    def details(self, fault):
        return [process_pb.ProcessSelector(selector=Oneof('tag', 'detail 中文'))] if self.fault == fault else []

    def events(self, response, name, label):
        self.stream_id += 1
        stream_id = self.stream_id
        pid = 999 if name == 'unknown' else self.pids[name]
        if self.fault == 'same-pid' and name == 'other' and label == 'start':
            pid = self.pids['first']
        def event(field, data):
            return response(event=process_pb.ProcessEvent(event=Oneof(field, data)))
        first = event('start', process_pb.ProcessEvent.StartEvent(pid=pid))
        ready = event('data', process_pb.ProcessEvent.DataEvent(output=Oneof('stdout', probe.READY)))
        def finish():
            for channel, raw in [('stdout', f'0 {hashlib.sha256(b"").hexdigest()}\n'.encode()),
                                 ('stderr', 'paired stderr 中文\n'.encode())]:
                yield event('data', process_pb.ProcessEvent.DataEvent(output=Oneof(channel, raw)))
            if self.fault != 'missing-end':
                yield event('end', process_pb.ProcessEvent.EndEvent(exit_code=0, exited=True))
        async def asynchronous():
            try:
                yield first
                if label == 'start':
                    yield ready
                await asyncio.wait_for(self.done.get(name, asyncio.Event()).wait(), 5)
                for item in finish():
                    yield item
            finally:
                self.closed.append((stream_id, name, label))
        def synchronous():
            try:
                yield first
                if label == 'start':
                    yield ready
                assert self.done[name].is_set(), 'terminal read before original PID input EOF'
                yield from finish()
            finally:
                self.closed.append((stream_id, name, label))
        return asynchronous() if self.asynchronous else synchronous()

    def listing(self, request, **kwargs):
        self.calls.append(('list', request, kwargs))
        items = []
        for name in self.lists.popleft():
            metadata = dict(self.specs[name]['metadata'])
            tag = metadata.pop('tag')
            if self.fault == 'metadata':
                tag += 'wrong'
            items.append(process_pb.ProcessInfo(pid=self.pids[name], tag=tag,
                                               config=process_pb.ProcessConfig(**metadata)))
        return process_pb.ListResponse(processes=items)

    def close(self, request, **kwargs):
        self.calls.append(('close', request, kwargs))
        assert request.process.selector.field == 'pid'
        if self.fault == 'unknown-eof':
            raise TimeoutError('EOF outcome unknown')
        name = next(name for name, pid in self.pids.items() if pid == request.process.selector.value)
        self.done[name].set()
        return process_pb.CloseStdinResponse()

    def signal(self, request, **kwargs):
        assert request.process.selector.field == 'pid'
        pid = request.process.selector.value
        self.signals.append(pid)
        if self.fault == 'kill':
            raise RuntimeError('signal outcome unknown')
        name = next(name for name, original in self.pids.items() if original == pid)
        self.done[name].set()
        return process_pb.SendSignalResponse()


class TagPolicyTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, kind, user='ubuntu', outcomes=None, fault=None):
        if outcomes is None:
            outcomes = ['first', 'other', 'duplicate' if kind == 'go' else 'first', 'other']
        backend = Transcript(asynchronous, kind, user, outcomes, fault)
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=backend.start, connect=backend.connect,
            list=wrap(side_effect=backend.listing), close_stdin=wrap(side_effect=backend.close),
            send_signal=wrap(side_effect=backend.signal))
        module = async_command if asynchronous else sync_command
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            commands = module.Commands('http://example.invalid', ConnectionConfig(api_key='synthetic', retries=0),
                                       Version('0.9.0'), Mock())
        commands.run = wrap(return_value=CommandResult(stdout='', stderr='', exit_code=0, error=None))
        sandbox = SimpleNamespace(commands=commands, files=SimpleNamespace(
            read=wrap(return_value='once\n'), exists=wrap(return_value=fault == 'effect')))
        return Calls(sandbox, asynchronous), backend

    async def test_both_policies_use_same_start_and_tag_inputs_with_original_handles(self):
        for asynchronous in (False, True):
            for user in probe.TAGS:
                requests = []
                for kind in ('go', 'rust'):
                    observed, backend = self.fixture(asynchronous, kind, user)
                    row = await probe.policy_case(observed, kind, user, PARENT)
                    self.assertEqual((row['status'], row['kind']), ('complete', kind))
                    self.assertEqual(backend.attempts, list(probe.NAMES))
                    self.assertFalse(backend.lists)
                    self.assertFalse(backend.outcomes)
                    self.assertEqual(backend.signals, [])
                    self.assertEqual(len(backend.closed), 7 if kind == 'go' else 6)
                    self.assertEqual(set(row['pids']), set(probe.NAMES if kind == 'go' else probe.NAMES[:2]))
                    requests.append([(method, request.to_json(), kwargs) for method, request, kwargs in backend.calls
                                     if method in ('start', 'connect')])
                    for method, request, kwargs in backend.calls:
                        if method == 'close':
                            self.assertEqual(kwargs['timeout_ms'], 15000)
                            self.assertEqual(request.process.selector.field, 'pid')
                    duplicate = [item for item in row['calls'] if item['method'] == 'process.start'][-1]
                    self.assertEqual('error' in duplicate, kind == 'rust')
                    if kind == 'rust':
                        self.assertEqual(duplicate['error']['rpc_code'], Code.ALREADY_EXISTS.value)
                        self.assertEqual(duplicate['error']['rpc_details'], [])
                self.assertEqual(requests[0], requests[1])

    async def test_fixed_go_outcomes_are_recorded_without_hiding_misses_or_duplicate_selection(self):
        for asynchronous in (False, True):
            for outcomes in ([None, None, None, None], ['first', 'other', 'first', 'other'],
                             [None, 'other', 'duplicate', None]):
                observed, backend = self.fixture(asynchronous, 'go', outcomes=outcomes)
                row = await probe.policy_case(observed, 'go', 'ubuntu', PARENT)
                calls = [item for item in row['calls'] if item['method'] == 'process.connect']
                for expected, operation in zip(outcomes, calls):
                    if expected is None:
                        self.assertEqual(operation['error']['rpc_code'], Code.NOT_FOUND.value)
                    else:
                        self.assertEqual(operation['result']['pid'], backend.pids[expected])
                self.assertEqual(backend.signals, [])

    async def test_unexpected_outcomes_never_signal_a_connect_reply_or_retry_start(self):
        cases = [('rust', [None], None, 2, [4322, 4321]),
                 ('rust', ['unknown'], None, 2, [4322, 4321]),
                 ('go', ['other'], None, 2, [4322, 4321]),
                 ('go', None, 'go-rejected', 3, [4322, 4321]),
                 ('rust', None, 'rust-executed', 3, [4323, 4322, 4321]),
                 ('go', None, 'unknown-start', 1, []),
                 ('go', None, 'unknown-duplicate', 3, [4322, 4321]),
                 ('go', None, 'same-pid', 2, [4321]),
                 ('go', None, 'wrong-error', 2, [4322, 4321]),
                 ('go', None, 'details', 2, [4322, 4321]),
                 ('rust', None, 'duplicate-details', 3, [4322, 4321]),
                 ('go', None, 'metadata', 2, [4322, 4321]),
                 ('rust', None, 'effect', 3, []),
                 ('rust', None, 'unknown-eof', 3, [4322, 4321])]
        for asynchronous in (False, True):
            for kind, outcomes, fault, attempts, signals in cases:
                observed, backend = self.fixture(asynchronous, kind, outcomes=outcomes, fault=fault)
                with self.subTest(asynchronous=asynchronous, kind=kind, fault=fault, outcomes=outcomes):
                    with self.assertRaises(Exception):
                        await probe.policy_case(observed, kind, 'ubuntu', PARENT)
                    self.assertEqual(backend.signals, signals)
                    self.assertEqual(len(backend.attempts), attempts)
                    self.assertEqual(observed.rows[0]['status'], 'incomplete')
                    self.assertEqual(len(backend.closed), len(set(backend.closed)))
                    if fault in ('wrong-error', 'details', 'duplicate-details'):
                        error = next(call['error'] for call in reversed(observed.rows[0]['calls'])
                                     if 'error' in call)
                        self.assertEqual(error['rpc_code'], backend.rpc_failure.code.value)
                        self.assertEqual(error['rpc_details'], [dict(type_name=part.type_name,
                            bytes=probe.value(part.message_bytes)) for part in backend.rpc_failure.details])

    async def test_truncated_end_and_cleanup_errors_keep_original_responsibilities(self):
        for asynchronous in (False, True):
            for fault in ('missing-end', 'close', 'kill'):
                outcomes = ['unknown'] if fault == 'kill' else None
                observed, backend = self.fixture(asynchronous, 'rust', outcomes=outcomes, fault=fault)
                original = probe.RecordedHandle.disconnect
                async def disconnect(handle):
                    await original(handle)
                    if fault == 'close' and handle.label == 'distinct-first':
                        raise RuntimeError('public subscription close uncertain')
                with self.subTest(asynchronous=asynchronous, fault=fault), self.assertRaises(Exception) as raised:
                    with patch.object(probe.RecordedHandle, 'disconnect', disconnect):
                        await probe.policy_case(observed, 'rust', 'ubuntu', PARENT)
                self.assertEqual(backend.signals, [4322, 4321])
                self.assertEqual(observed.rows[0]['status'], 'incomplete')
                if fault == 'kill':
                    def flatten(error):
                        return [part for child in error.exceptions for part in flatten(child)] if isinstance(error, BaseExceptionGroup) else [error]
                    errors = flatten(raised.exception)
                    self.assertEqual(sum(isinstance(error, RuntimeError) for error in errors), 2)
                    self.assertTrue(any(isinstance(error, AssertionError) for error in errors))
