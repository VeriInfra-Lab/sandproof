"""Original SDK argument-probe ownership faults, without a product guest."""
import asyncio
from contextlib import ExitStack
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.exceptions import TimeoutException
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
import test_command_probe_cleanup as wire
import sdk_command_arguments as probe

DIRECTORY = '/tmp/command-arguments-fixed'


def terminal(directory=DIRECTORY, user='ubuntu', *, stdout=None, stderr='', code=0, error=None):
    text = directory + '\n' + user + '\nspaces and 中文\n' if stdout is None else stdout
    events = wire.end(text, code=code, error=error)
    if stderr:
        events.insert(0, ('data', wire.pb.ProcessEvent.DataEvent(output=Oneof('stderr', stderr.encode()))))
    return events


def rejection(index):
    return ConnectError(Code.UNAUTHENTICATED if index == 0 else Code.INVALID_ARGUMENT, 'prescribed refusal')


class ArgumentOwnershipTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.stack.enter_context(patch.object(probe.uuid, 'uuid4', return_value=SimpleNamespace(hex='fixed')))

    def rejected(self, asynchronous, background, script):
        # Reach the first tested rejection using original RPC Start responses.
        # Legal context calls have their own original-SDK tests below.
        starts = [(100, wire.end(''))]
        if background:
            starts.extend(rejection(index) for index in range(6))
        starts.append(script)
        tape = wire.Transcript(asynchronous, starts)
        wrap = AsyncMock if asynchronous else Mock
        tape.sandbox.files.write = wrap(return_value=None)
        tape.sandbox.files.exists.return_value = False
        return tape

    async def test_valid_context_uses_original_inputs_and_closes_each_subscription(self):
        for asynchronous in (False, True):
            for background in (False, True):
                for user in ('root', 'ubuntu'):
                    tape = wire.Transcript(asynchronous, [(4321, terminal(user=user))])
                    await probe.healthy_context(tape.sandbox, DIRECTORY, user, background)
                    self.assertEqual(tape.signals(), [])
                    self.assertEqual(len(tape.closed), 1)
                    request, options = tape.calls[0][1:]
                    self.assertEqual(request.process.cwd, DIRECTORY)
                    self.assertEqual(dict(request.process.envs), {'SP_ARGUMENT_VALUE': 'spaces and 中文'})
                    self.assertEqual(options['timeout_ms'], 15000)
                    self.assertEqual(list(request.process.args), ['-l', '-c',
                        "pwd; id -un; printf '%s\\n' \"$SP_ARGUMENT_VALUE\""])

    async def test_wait_failure_and_cancellation_signal_only_original_pid_and_close(self):
        for asynchronous in (False, True):
            for fault in ('missing-end', 'deadline', 'cancel'):
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    events = [wire.deadline()] if fault == 'deadline' else []
                    tape = wire.Transcript(asynchronous, [(4321, events)])
                    with ExitStack() as stack:
                        if fault == 'cancel':
                            module = wire.async_command if asynchronous else wire.sync_command
                            cls = module.AsyncCommandHandle if asynchronous else module.CommandHandle
                            stack.enter_context(patch.object(cls, 'wait', AsyncMock(side_effect=asyncio.CancelledError())))
                        expected = {'cancel': asyncio.CancelledError, 'deadline': TimeoutException,
                                    'missing-end': Exception}[fault]
                        with self.assertRaises(expected) as caught:
                            await probe.healthy_context(tape.sandbox, DIRECTORY, 'ubuntu', True)
                        self.assertIs(type(caught.exception), expected)
                        if fault == 'missing-end':
                            self.assertEqual(str(caught.exception), 'Command ended without an end event')
                    self.assertEqual(tape.signals(), [Oneof('pid', 4321)])
                    self.assertEqual(len(tape.closed), 1)

    async def test_wrong_terminal_is_failure_without_post_end_signal(self):
        for asynchronous in (False, True):
            for background in (False, True):
                for fault in ('stdout', 'stderr', 'exit', 'error'):
                    with self.subTest(asynchronous=asynchronous, background=background, fault=fault):
                        events = terminal(stdout='bad' if fault == 'stdout' else None,
                            stderr='bad' if fault == 'stderr' else '', code=17 if fault == 'exit' else 0,
                            error='unexpected error' if fault == 'error' else None)
                        tape = wire.Transcript(asynchronous, [(4321, events)])
                        with self.assertRaises(Exception):
                            await probe.healthy_context(tape.sandbox, DIRECTORY, 'ubuntu', background)
                        self.assertEqual(tape.signals(), [])
                        self.assertEqual(len(tape.closed), 1)

    async def test_unexpected_rejection_success_owns_only_valid_original_pid(self):
        for asynchronous in (False, True):
            for pid in (4321, 0, -1):
                with self.subTest(asynchronous=asynchronous, pid=pid):
                    tape = self.rejected(asynchronous, True, (pid, None))
                    with patch.object(probe, 'healthy_context', AsyncMock()), self.assertRaises(AssertionError):
                        await probe.argument_cases(tape.sandbox)
                    self.assertEqual(tape.signals(), [Oneof('pid', 4321)] if pid == 4321 else [])
                    self.assertFalse(tape.starts)
                    self.assertEqual(len(tape.closed), 8)
            tape = wire.Transcript(asynchronous, [(0, terminal())])
            with self.assertRaises(AssertionError):
                await probe.healthy_context(tape.sandbox, DIRECTORY, 'ubuntu', True)
            self.assertEqual(tape.signals(), [])
            self.assertEqual(len(tape.closed), 1)

    async def test_unknown_start_never_replays_or_invents_identity(self):
        for asynchronous in (False, True):
            for background in (False, True):
                tape = self.rejected(asynchronous, background, wire.deadline())
                with patch.object(probe, 'healthy_context', AsyncMock()), self.assertRaises(AssertionError):
                    await probe.argument_cases(tape.sandbox)
                self.assertEqual(tape.signals(), [])
                self.assertFalse(tape.starts)
                self.assertEqual(len(tape.calls), 8 if background else 2)
                tape = wire.Transcript(asynchronous, [wire.deadline()])
                with self.assertRaises(Exception):
                    await probe.healthy_context(tape.sandbox, DIRECTORY, 'ubuntu', background)
                self.assertEqual([item[0] for item in tape.calls], ['start'])

    def disconnect_failure(self, asynchronous, error):
        module = wire.async_command if asynchronous else wire.sync_command
        cls = module.AsyncCommandHandle if asynchronous else module.CommandHandle
        original = cls.disconnect
        async def disconnect(handle):
            await wire.inputs.call(original, handle)
            raise error
        return patch.object(cls, 'disconnect', disconnect)

    async def test_rejection_signal_and_close_failures_keep_primary_and_all_cleanup(self):
        for asynchronous in (False, True):
            tape = self.rejected(asynchronous, True, (4321, None))
            signal_error, close_error = RuntimeError('unknown signal'), RuntimeError('unknown disconnect')
            tape.sandbox.commands._rpc.send_signal.side_effect = signal_error
            with patch.object(probe, 'healthy_context', AsyncMock()), self.disconnect_failure(asynchronous, close_error):
                with self.assertRaises(BaseExceptionGroup) as caught:
                    await probe.argument_cases(tape.sandbox)
            self.assertIsInstance(caught.exception.exceptions[0], AssertionError)
            self.assertEqual(list(caught.exception.exceptions[1:]), [signal_error, close_error])
            self.assertEqual(tape.sandbox.commands._rpc.send_signal.call_count, 1)
            self.assertEqual(len(tape.closed), 8)

    async def test_invalid_terminal_and_close_failure_are_both_retained(self):
        for asynchronous in (False, True):
            tape = wire.Transcript(asynchronous, [(4321, terminal(stdout='wrong'))])
            close_error = RuntimeError('unknown disconnect')
            with self.disconnect_failure(asynchronous, close_error), self.assertRaises(BaseExceptionGroup) as caught:
                await probe.healthy_context(tape.sandbox, DIRECTORY, 'ubuntu', True)
            self.assertIsInstance(caught.exception.exceptions[0], AssertionError)
            self.assertIs(caught.exception.exceptions[1], close_error)
            self.assertEqual(tape.signals(), [])
            self.assertEqual(len(tape.closed), 1)

    async def test_full_original_matrix_and_report_remain_intact(self):
        for asynchronous in (False, True):
            starts = [(100, wire.end('')), (101, terminal(DIRECTORY + '/private', 'root'))]
            for _background in (False, True):
                starts.extend([(200, terminal(user='root')), (201, terminal())])
                for index in range(6):
                    starts.extend([rejection(index), (300 + index, terminal())])
            tape = wire.Transcript(asynchronous, starts)
            wrap = AsyncMock if asynchronous else Mock
            tape.sandbox.files.write = wrap(return_value=None)
            tape.sandbox.files.exists.return_value = False
            tape.sandbox.files.read.return_value = 'ordinary file'
            rows = await probe.argument_cases(tape.sandbox)
            self.assertEqual(len(rows), 12)
            self.assertEqual([row['background'] for row in rows], [False] * 6 + [True] * 6)
            self.assertTrue(all(row['marker_absent'] and row['subsequent_command_healthy'] for row in rows))
            self.assertEqual(len(tape.calls), 30)
            self.assertFalse(tape.starts)
            self.assertEqual(len(tape.closed), 30)
            self.assertEqual(tape.signals(), [])
            self.assertEqual(tape.sandbox.files.exists.call_count, 12)
            self.assertEqual(tape.sandbox.files.read.call_count, 1)
