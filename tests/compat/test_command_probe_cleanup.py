"""Original SDK event transcripts and probe ownership faults; no product VM."""
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
from e2b.envd.process import process_pb as pb
from e2b.sandbox.commands.command_handle import CommandExitException
from e2b.sandbox_sync.commands import command as sync_command
from e2b.sandbox_async.commands import command as async_command
from packaging.version import Version
from protobuf import Oneof
from sdk_support import isolated_sdk_environment

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_command_input as inputs
import sdk_command_timeouts as deadlines


def end(stdout='finished\n', *, code=0, error=None):
    events = []
    if stdout:
        events.append(('data', pb.ProcessEvent.DataEvent(output=Oneof('stdout', stdout.encode()))))
    fields = dict(exit_code=code, exited=True)
    if error is not None:
        fields['error'] = error
    return events + [('end', pb.ProcessEvent.EndEvent(**fields))]


def deadline():
    return ConnectError(Code.DEADLINE_EXCEEDED, '')


class Transcript:
    """Prescribed Start/Connect events and List replies, never a guest registry."""
    def __init__(self, asynchronous, starts, connects=(), lists=()):
        self.asynchronous = asynchronous
        self.starts, self.connects, self.lists = map(deque, (starts, connects, lists))
        self.calls, self.closed = [], []
        wrap = AsyncMock if asynchronous else Mock
        rpc = SimpleNamespace(start=self.start, connect=self.connect,
            list=wrap(side_effect=self.list), send_signal=wrap(side_effect=self.signal),
            send_input=wrap(side_effect=self.input), close_stdin=wrap(side_effect=self.close))
        module = async_command if asynchronous else sync_command
        with patch.object(module, 'create_rpc_client', return_value=rpc):
            commands = module.Commands('http://example.invalid',
                ConnectionConfig(api_key='synthetic', retries=0), Version('0.9.0'), Mock())
        self.sandbox = SimpleNamespace(commands=commands,
            files=SimpleNamespace(exists=wrap(return_value=True), read=wrap(return_value='once\n')))

    def start(self, request, **kwargs):
        return self.stream('start', pb.StartResponse, self.starts.popleft(), request, kwargs)

    def connect(self, request, **kwargs):
        return self.stream('connect', pb.ConnectResponse, self.connects.popleft(), request, kwargs)

    def stream(self, name, response, script, request, kwargs):
        self.calls.append((name, request, kwargs))
        label = (name, len(self.calls))
        pid, events = (None, []) if isinstance(script, Exception) else script
        def messages():
            if isinstance(script, Exception):
                raise script
            yield response(event=pb.ProcessEvent(event=Oneof('start', pb.ProcessEvent.StartEvent(pid=pid))))
            for event in events or []:
                if isinstance(event, Exception):
                    raise event
                yield None if event is None else response(event=pb.ProcessEvent(event=Oneof(*event)))
        async def asynchronous():
            try:
                for message in messages():
                    if message is None:
                        await asyncio.Event().wait()
                    yield message
                if events is None:
                    # A held subscription ends only when the SDK disconnects.
                    await asyncio.Event().wait()
            finally:
                self.closed.append(label)
        def synchronous():
            try:
                for message in messages():
                    assert message is not None, 'unexpected read beyond held output'
                    yield message
                assert events is not None, 'unexpected wait on a held subscription'
            finally:
                self.closed.append(label)
        return asynchronous() if self.asynchronous else synchronous()

    def list(self, request, **kwargs):
        self.calls.append(('list', request, kwargs))
        rows = self.lists.popleft()
        if callable(rows):
            rows = rows(self.calls[0][1])
        processes = []
        for row in rows:
            row = dict(row)
            pid, tag = row.pop('pid'), row.pop('tag')
            if row['cwd'] is None:
                row.pop('cwd')
            fields = dict(pid=pid, config=pb.ProcessConfig(**row))
            if tag is not None:
                fields['tag'] = tag
            processes.append(pb.ProcessInfo(**fields))
        return pb.ListResponse(processes=processes)

    def signal(self, request, **kwargs):
        self.calls.append(('signal', request, kwargs))
        return pb.SendSignalResponse()

    def input(self, request, **kwargs):
        self.calls.append(('input', request, kwargs))
        return pb.SendInputResponse()

    def close(self, request, **kwargs):
        self.calls.append(('close', request, kwargs))
        return pb.CloseStdinResponse()

    def signals(self):
        return [row[1].process.selector for row in self.calls if row[0] == 'signal']


def original_row(request):
    return [deadlines.expected_entry(request.process.args[-1], 4321)]


class OriginalSdkTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    async def test_stdin_defaults_binary_reconnect_and_each_subscription_closed_once(self):
        payload = 'first line and 中文\n'.encode() + bytes(range(256)) * 257
        output = f'{len(payload)} {hashlib.sha256(payload).hexdigest()}\n'
        for asynchronous in (False, True):
            tape = Transcript(asynchronous, [(4000, end(inputs.EMPTY)), (4001, end(inputs.EMPTY)),
                (4321, end(inputs.EMPTY)), (4322, None)], [(4322, end(output))])
            rows = await inputs.user_cases(tape.sandbox, 'ubuntu')
            self.assertEqual([row['received_bytes'] for row in rows], [0, 0, 0, len(payload)])
            self.assertEqual(tape.signals(), [])
            self.assertEqual(len(tape.closed), 5)
            self.assertEqual(len(set(tape.closed)), 5)
            sent = [row[1].input.input.value for row in tape.calls if row[0] == 'input']
            self.assertEqual(b''.join(sent), payload)
            self.assertEqual([row[1].process.selector for row in tape.calls if row[0] == 'connect'],
                             [Oneof('pid', 4322)])

    async def test_stdin_terminal_mismatch_or_marker_failure_never_signals_after_end(self):
        for asynchronous in (False, True):
            for fault in ('output', 'exit', 'error', 'marker'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    terminal = end('wrong' if fault == 'output' else inputs.EMPTY,
                                   code=17 if fault == 'exit' else 0,
                                   error='unexpected' if fault == 'error' else None)
                    tape = Transcript(asynchronous, [(4000, end(inputs.EMPTY)),
                        (4001, end(inputs.EMPTY)), (4321, terminal)])
                    if fault == 'marker':
                        tape.sandbox.files.read.side_effect = ValueError('marker read failed')
                    with self.assertRaises(Exception):
                        await inputs.user_cases(tape.sandbox, 'root')
                    self.assertEqual(tape.signals(), [])
                    self.assertEqual(len(tape.closed), 3)

    async def test_run_actual_end_and_missing_end_keep_distinct_cleanup(self):
        for asynchronous in (False, True):
            for fault in ('valid', 'output', 'exit', 'error', 'marker', 'missing'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    terminal = [] if fault == 'missing' else end(
                        'wrong' if fault == 'output' else 'finished\n',
                        code=17 if fault == 'exit' else 0,
                        error='unexpected' if fault == 'error' else None)
                    tape = Transcript(asynchronous, [(4321, terminal)])
                    if fault == 'marker':
                        tape.sandbox.files.read.side_effect = ValueError('marker read failed')
                    invoke = deadlines.run_case(tape.sandbox, asynchronous, 'timeout', 'zero', 0, True)
                    if fault == 'valid':
                        row = await invoke
                        self.assertTrue(row['terminal_observed'])
                        self.assertEqual(row['original_pid'], 4321)
                    else:
                        with self.assertRaises(Exception):
                            await invoke
                    self.assertEqual(tape.signals(), [Oneof('pid', 4321)] if fault == 'missing' else [])
                    self.assertEqual(len(tape.closed), 1)

    async def test_connect_recovery_and_wait_deadline_accept_delayed_retirement_without_kill(self):
        for asynchronous in (False, True):
            for mode in ('connect', 'wait-connect', 'wait-start'):
                with self.subTest(asynchronous=asynchronous, mode=mode):
                    killed = end('', code=-1, error='signal: killed')
                    starts = [(4321, killed if mode == 'wait-start' else None)]
                    connects = [(4321, killed)] if mode == 'wait-start' else [
                        (4321, [deadline()]), (4321, end())]
                    if mode.startswith('wait'):
                        starts.append((5000, end('healthy')))
                    tape = Transcript(asynchronous, starts, connects,
                        [original_row, []] if mode == 'wait-start' else [original_row])
                    if mode == 'connect':
                        row = await deadlines.connect_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1)
                    else:
                        row = await deadlines.wait_deadline_case(tape.sandbox, mode == 'wait-start')
                    self.assertEqual(tape.signals(), [])
                    self.assertEqual(len(tape.closed), 3 if mode != 'wait-connect' else 4)
                    self.assertEqual(len(tape.closed), len(set(tape.closed)))
                    if mode == 'wait-start':
                        self.assertEqual(len(row['post_end_list_observations']), 2)
                        self.assertEqual(row['post_end_list_observations'][-1]['original_entry'], [])

    async def test_unknown_start_candidates_are_exact_read_only_and_start_is_not_replayed(self):
        def candidates(request):
            command = request.process.args[-1]
            rows = [deadlines.expected_entry(command, pid) for pid in (4321, 4322)]
            for index, (field, value) in enumerate((('tag', 'other'), ('cmd', 'other'),
                    ('args', ['-l', '-c', 'prefix ' + command]), ('envs', {'X': 'Y'}), ('cwd', '/tmp'))):
                rows.append(dict(deadlines.expected_entry(command, 6000 + index), **{field: value}))
            return rows
        for asynchronous in (False, True):
            for background in (False, True):
                tape = Transcript(asynchronous, [deadline()], lists=[candidates])
                row = await deadlines.run_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1, background)
                self.assertEqual([entry['pid'] for entry in row['unacknowledged_candidates']], [4321, 4322])
                self.assertFalse(row['terminal_observed'])
                self.assertIsNone(row['original_pid'])
                self.assertEqual([item[0] for item in tape.calls], ['start', 'list'])
                self.assertEqual(tape.signals(), [])

    async def test_known_stream_deadline_signals_once_but_unexpected_end_does_not(self):
        for asynchronous in (False, True):
            for terminal in (False, True):
                tape = Transcript(asynchronous, [(4321, end() if terminal else [deadline()])])
                invoke = deadlines.run_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1, True)
                if terminal:
                    with self.assertRaisesRegex(AssertionError, 'ignored'):
                        await invoke
                else:
                    row = await invoke
                    self.assertFalse(row['terminal_observed'])
                    self.assertEqual(row['original_pid'], 4321)
                    self.assertIsNone(row['unacknowledged_candidates'])
                self.assertEqual(tape.signals(), [] if terminal else [Oneof('pid', 4321)])
                self.assertEqual(len(tape.closed), 1)

    async def test_connect_wrong_pid_and_unknown_input_keep_original_identity(self):
        for asynchronous in (False, True):
            for fault in ('wrong-pid', 'unknown-input', 'completed'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    tape = Transcript(asynchronous, [(4321, None)],
                        [(9999 if fault == 'wrong-pid' else 4321, end())], [original_row])
                    if fault == 'unknown-input':
                        wrap = AsyncMock if asynchronous else Mock
                        tape.sandbox.commands._rpc.close_stdin = wrap(side_effect=deadline())
                    invoke = deadlines.connect_case(tape.sandbox, asynchronous, 'timeout', 'zero', 0)
                    if fault == 'completed':
                        await invoke
                    else:
                        with self.assertRaises(Exception):
                            await invoke
                    self.assertEqual(tape.signals(), [] if fault == 'completed' else [Oneof('pid', 4321)])
                    self.assertEqual(len(tape.closed), 2)
                    self.assertEqual([row[1].process.selector for row in tape.calls if row[0] == 'connect'],
                                     [Oneof('pid', 4321)])


class OwnershipTests(unittest.IsolatedAsyncioTestCase):
    def handle(self, pid=4321, **kwargs):
        return SimpleNamespace(pid=pid, disconnect=AsyncMock(),
            wait=AsyncMock(**kwargs))

    async def test_wrong_connect_identity_is_closed_but_only_original_is_signalled(self):
        for pid in (4322, True, 0, -1, 4194305):
            original, wrong = self.handle(), self.handle(pid)
            commands = SimpleNamespace(kill=AsyncMock())
            with self.assertRaises(AssertionError):
                async with inputs.KnownCommand(commands, original) as owned:
                    await owned.disconnect(original)
                    owned.add(wrong)
            commands.kill.assert_awaited_once_with(4321, request_timeout=15)
            original.disconnect.assert_awaited_once()
            wrong.disconnect.assert_awaited_once()

    async def test_invalid_start_identity_only_closes_returned_subscription(self):
        for pid in (True, None, 0, -1, 4194305):
            original, commands = self.handle(pid), SimpleNamespace(kill=AsyncMock())
            with self.assertRaises(AssertionError):
                async with inputs.KnownCommand(commands, original):
                    self.fail('invalid PID was accepted')
            commands.kill.assert_not_awaited()
            original.disconnect.assert_awaited_once()

    async def test_primary_signal_and_all_close_failures_survive_without_retries(self):
        original, resumed = self.handle(), self.handle()
        failures = [ValueError('primary'), RuntimeError('signal'),
                    RuntimeError('resumed close'), RuntimeError('original close')]
        commands = SimpleNamespace(kill=AsyncMock(side_effect=failures[1]))
        original.disconnect.side_effect, resumed.disconnect.side_effect = failures[3], failures[2]
        owned = inputs.KnownCommand(commands, original)
        with self.assertRaises(BaseExceptionGroup) as caught:
            async with owned:
                owned.add(resumed)
                owned.add(resumed)
                raise failures[0]
        self.assertEqual(list(caught.exception.exceptions), failures)
        await owned.cleanup(None)
        commands.kill.assert_awaited_once_with(4321, request_timeout=15)
        original.disconnect.assert_awaited_once()
        resumed.disconnect.assert_awaited_once()

    async def test_unknown_explicit_disconnect_is_not_retried_and_cancellation_is_preserved(self):
        for failure in (RuntimeError('unknown close'), asyncio.CancelledError()):
            original, commands = self.handle(), SimpleNamespace(kill=AsyncMock())
            original.disconnect.side_effect = failure
            with self.assertRaises(type(failure)) as caught:
                async with inputs.KnownCommand(commands, original) as owned:
                    await owned.disconnect(original)
            self.assertIs(caught.exception, failure)
            original.disconnect.assert_awaited_once()
            commands.kill.assert_awaited_once_with(4321, request_timeout=15)


class ObservationTests(unittest.IsolatedAsyncioTestCase):
    def sandbox(self, rows):
        return SimpleNamespace(commands=SimpleNamespace(list=AsyncMock(return_value=[
            SimpleNamespace(**entry) for entry in rows])))

    async def test_bounded_list_observation_rejects_invalid_duplicate_large_or_changed_rows(self):
        good = deadlines.expected_entry('command', 4321)
        bad_rows = [[dict(good, pid=True)], [dict(good, pid=0)], [good, good],
                    [dict(good, pid=pid + 1) for pid in range(129)],
                    [dict(good, args=['x' * (128 * 1024)])], [dict(good, cwd='/changed')]]
        for rows in bad_rows:
            with self.subTest(rows=len(rows)), self.assertRaises(AssertionError):
                await deadlines.observe_absent(self.sandbox(rows), 4321, 'command')

    async def test_deadline_and_count_limits_do_not_accept_late_absence(self):
        clock = Mock(side_effect=[0, 1, 15])
        sandbox = self.sandbox([])
        with self.assertRaisesRegex(AssertionError, 'deadline'):
            await deadlines.observe_absent(sandbox, 4321, 'command', clock=clock)
        sandbox.commands.list.assert_awaited_once_with(request_timeout=14)
        sandbox = self.sandbox([deadlines.expected_entry('command', 4321)])
        pause = AsyncMock()
        with self.assertRaisesRegex(AssertionError, 'count exceeded'):
            await deadlines.observe_absent(sandbox, 4321, 'command', clock=lambda: 0, pause=pause)
        self.assertEqual(sandbox.commands.list.await_count, 128)
        self.assertEqual(pause.await_count, 128)
        pause = AsyncMock()
        with self.assertRaisesRegex(AssertionError, 'deadline'):
            await deadlines.observe_absent(sandbox, 4321, 'command',
                clock=Mock(side_effect=[0, 1, 14, 16]), pause=pause)
        pause.assert_not_awaited()

    def test_unexpected_errors_and_process_end_are_not_accepted_as_stream_deadlines(self):
        for error in (ValueError('deadline'), deadlines.TimeoutException('wrong mapping'),
                      CommandExitException('', '', -1, 'signal: killed')):
            with self.assertRaises(AssertionError):
                deadlines.verify_failure(error, 'stream_deadline')
