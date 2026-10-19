"""Fixed SDK unary transcripts with explicit End release; no guest registry/VM."""
import asyncio
from contextlib import ExitStack
import subprocess
import sys
import unittest
from unittest.mock import patch

from connectrpc.code import Code
from connectrpc.errors import ConnectError
from e2b.envd.process import process_pb as pb
from e2b.sandbox.commands.command_handle import CommandExitException
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
import test_command_probe_cleanup as wire
import sdk_rpc_timeouts as probe

OPERATIONS = ('list', 'kill', 'send_stdin', 'close_stdin', 'handle.send_stdin', 'handle.close_stdin')


class UnaryTranscript(wire.Transcript):
    def __init__(self, asynchronous, operation, case, *, deadline=False, effect=False, listed=True):
        self.target = {'list': 'list', 'kill': 'signal', 'send_stdin': 'input', 'close_stdin': 'close'}[
            operation.removeprefix('handle.')]
        self.case, self.deadline, self.effect, self.listed = case, deadline, effect, listed
        self.tested, self.released, self.gate = False, False, asyncio.Event()
        self.operation_error = self.cleanup_error = None
        self.received = b''
        if (self.target == 'input' and case != 'above_rpc_limit' and (not deadline or effect)
                or self.target in ('signal', 'close') and case == 'above_rpc_limit'):
            self.received = probe.INPUT
        terminal = wire.end(self.received.hex() + '\n')
        if self.target == 'signal' and case != 'above_rpc_limit':
            terminal = wire.end('', code=-1, error='signal: killed')
        super().__init__(asynchronous, [(4321, terminal), (5000, wire.end('')), (5001, wire.end('healthy'))])

    def stream(self, name, response, script, request, kwargs):
        if not isinstance(script, tuple) or script[0] != 4321:
            return super().stream(name, response, script, request, kwargs)
        self.calls.append((name, request, kwargs))
        label, (pid, events) = (name, len(self.calls)), script
        first = response(event=pb.ProcessEvent(event=Oneof('start', pb.ProcessEvent.StartEvent(pid=pid))))
        def terminal():
            for field, payload in events:
                yield response(event=pb.ProcessEvent(event=Oneof(field, payload)))
        def synchronous():
            try:
                yield first
                assert self.released, 'End must follow a prescribed control boundary'
                yield from terminal()
            finally:
                self.closed.append(label)
        async def async_events():
            try:
                yield first
                await self.gate.wait()
                for event in terminal():
                    yield event
            finally:
                self.closed.append(label)
        return async_events() if self.asynchronous else synchronous()

    def release(self):
        self.released = True
        if self.asynchronous:
            self.gate.set()

    def response(self, name, request, kwargs):
        self.calls.append((name, request, kwargs))
        if name != 'list':
            assert request.process.selector == Oneof('pid', 4321)
        if name == self.target and not self.tested:
            self.tested = True
            if self.operation_error is not None:
                raise self.operation_error
            if self.case == 'above_rpc_limit':
                raise ConnectError(Code.INVALID_ARGUMENT, 'parse timeout: "10000000000" has >10 digits')
            if self.deadline:
                if self.effect and name in ('signal', 'close'):
                    self.release()
                raise wire.deadline()
        if name == 'signal' and self.cleanup_error is not None:
            raise self.cleanup_error
        if name in ('signal', 'close'):
            self.release()
        responses = {'input': pb.SendInputResponse, 'close': pb.CloseStdinResponse, 'signal': pb.SendSignalResponse}
        if name == 'list':
            entries = [pb.ProcessInfo(pid=4321, config=pb.ProcessConfig(cmd='original'))] if self.listed else []
            return pb.ListResponse(processes=entries)
        return responses[name]()

    def list(self, request, **kwargs):
        return self.response('list', request, kwargs)

    def signal(self, request, **kwargs):
        return self.response('signal', request, kwargs)

    def input(self, request, **kwargs):
        return self.response('input', request, kwargs)

    def close(self, request, **kwargs):
        return self.response('close', request, kwargs)


class PrivateProgramTests(unittest.TestCase):
    def test_original_quiet_program_has_exact_empty_and_binary_output(self):
        for data in (b'', probe.INPUT, bytes(range(256))):
            completed = subprocess.run([sys.executable, '-I', '-c', probe.QUIET], input=data,
                                       capture_output=True, timeout=5, check=True)
            self.assertEqual((completed.stdout, completed.stderr), ((data.hex() + '\n').encode(), b''))


class OriginalSdkTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    async def test_all_operations_preserve_parameters_validate_end_and_avoid_extra_signals(self):
        for asynchronous in (False, True):
            for operation in OPERATIONS:
                for case, value in probe.VALUES:
                    with self.subTest(asynchronous=asynchronous, operation=operation, case=case):
                        tape = UnaryTranscript(asynchronous, operation, case)
                        row = await probe.command_case(tape.sandbox, operation, case, value)
                        self.assertEqual(row['outcome'], 'invalid_rpc_timeout' if case == 'above_rpc_limit' else 'completed')
                        self.assertTrue(row['terminal_observed'])
                        self.assertTrue(row['observed_listed'])  # Deliberately retain End's List entry.
                        self.assertEqual(row['original_pid'], 4321)
                        self.assertEqual(row['rejection_recovered'], tape.target in ('signal', 'close') and case == 'above_rpc_limit')
                        self.assertEqual(len(tape.signals()), int(operation == 'kill'))
                        self.assertEqual(row['signal_attempt_source'], 'operation' if operation == 'kill' else None)
                        self.assertEqual(len(tape.closed), 3)
                        sent = [item[1].input.input.value for item in tape.calls if item[0] == 'input']
                        self.assertEqual(sent, [probe.INPUT] if tape.target == 'input' or row['rejection_recovered'] else [])
                        target = next(item for item in tape.calls if item[0] == tape.target)
                        self.assertEqual(target[2]['timeout_ms'], {'omitted': 60000, 'zero': None,
                            'negative': 1, 'above_rpc_limit': 10000000000}[case])
                        starts = [item for item in tape.calls if item[0] == 'start']
                        self.assertEqual(len(starts), 3)
                        self.assertEqual(starts[0][2]['timeout_ms'], 60000)
                        self.assertTrue(starts[1][1].process.args[-1].startswith('while [ ! -s '))
                        self.assertEqual(starts[2][1].process.args[-1], 'printf healthy')

    async def test_unknown_unary_results_are_not_replayed_or_promoted_by_list_membership(self):
        for asynchronous in (False, True):
            for operation in OPERATIONS:
                for effect in (False, True):
                    with self.subTest(asynchronous=asynchronous, operation=operation, effect=effect):
                        tape = UnaryTranscript(asynchronous, operation, 'negative', deadline=True, effect=effect)
                        row = await probe.command_case(tape.sandbox, operation, 'negative', -1)
                        self.assertEqual(row['outcome'], 'deadline')
                        self.assertEqual(len(tape.signals()), int(tape.target in ('signal', 'close')))
                        self.assertEqual(row['terminal_observed'], tape.target in ('list', 'input'))
                        if tape.target == 'signal':
                            self.assertEqual(row['signal_attempt_source'], 'operation')
                        elif tape.target == 'close':
                            self.assertEqual(row['signal_attempt_source'], 'cleanup')
                        else:
                            self.assertIsNone(row['signal_attempt_source'])
                        mutations = [item for item in tape.calls if item[0] == tape.target]
                        if tape.target != 'list':
                            self.assertEqual(len(mutations), 1)
                        if tape.target == 'input':
                            self.assertEqual(row['received_bytes'], len(probe.INPUT) if effect else 0)
                            sent = [item[1].input.input.value for item in tape.calls if item[0] == 'input']
                            self.assertEqual(sent, [probe.INPUT])
                        self.assertEqual(len(tape.closed), 3)
        for asynchronous in (False, True):
            tape = UnaryTranscript(asynchronous, 'kill', 'negative', deadline=True, effect=True, listed=False)
            row = await probe.command_case(tape.sandbox, 'kill', 'negative', -1)
            self.assertFalse(row['observed_listed'])
            self.assertFalse(row['terminal_observed'])
            self.assertEqual(row['outcome'], 'deadline')
            self.assertEqual(len(tape.signals()), 1)

    async def test_rejected_controls_require_original_input_result_despite_stale_list_entry(self):
        for asynchronous in (False, True):
            for operation in ('kill', 'close_stdin', 'handle.close_stdin'):
                with self.subTest(asynchronous=asynchronous, operation=operation):
                    tape = UnaryTranscript(asynchronous, operation, 'above_rpc_limit')
                    # Prescribe the outcome of an incorrectly applied control:
                    # List still includes the PID but its original stream cannot
                    # confirm the distinct input sent after the header rejection.
                    tape.starts[0] = (4321, wire.end('', code=-1, error='signal: killed')
                                      if operation == 'kill' else wire.end('\n'))
                    with self.assertRaises(Exception):
                        await probe.command_case(tape.sandbox, operation, 'above_rpc_limit', 10000000)
                    self.assertEqual(len(tape.signals()), int(operation == 'kill'))
                    self.assertEqual([item[1].input.input.value for item in tape.calls if item[0] == 'input'], [probe.INPUT])
                    self.assertEqual([item[0] for item in tape.calls].count('start'), 2)

    async def test_terminal_corruption_and_marker_failure_never_trigger_a_post_end_kill(self):
        for asynchronous in (False, True):
            for operation in ('kill', 'send_stdin', 'close_stdin'):
                for fault in ('stdout', 'stderr', 'exit', 'error', 'marker'):
                    with self.subTest(asynchronous=asynchronous, operation=operation, fault=fault):
                        tape = UnaryTranscript(asynchronous, operation, 'zero')
                        if fault == 'marker':
                            tape.sandbox.files.read.side_effect = ValueError('marker failed')
                        else:
                            terminal = wire.end('unexpected' if fault == 'stdout' else tape.received.hex() + '\n',
                                code=17 if fault == 'exit' else (-1 if operation == 'kill' else 0),
                                error='wrong' if fault == 'error' else ('signal: killed' if operation == 'kill' else None))
                            if operation == 'kill' and fault != 'stdout':
                                terminal = [event for event in terminal if event[0] != 'data']
                            if fault == 'stderr':
                                terminal.insert(0, ('data', pb.ProcessEvent.DataEvent(output=Oneof('stderr', b'wrong'))))
                            tape.starts[0] = (4321, terminal)
                        with self.assertRaises(Exception):
                            await probe.command_case(tape.sandbox, operation, 'zero', 0)
                        self.assertEqual(len(tape.signals()), int(operation == 'kill'))
                        self.assertEqual(len(tape.closed), 2)

    async def test_hex_whitespace_and_duplicate_input_are_not_normalized_into_success(self):
        for asynchronous in (False, True):
            for stdout in (probe.INPUT.hex() + '\n\n', ' ' + probe.INPUT.hex() + '\n',
                           ' '.join(probe.INPUT.hex()) + '\n', (probe.INPUT * 2).hex() + '\n'):
                tape = UnaryTranscript(asynchronous, 'send_stdin', 'negative', deadline=True, effect=True)
                tape.starts[0] = (4321, wire.end(stdout))
                with self.assertRaises(AssertionError):
                    await probe.command_case(tape.sandbox, 'send_stdin', 'negative', -1)
                self.assertEqual(tape.signals(), [])

    async def test_missing_end_and_unknown_start_preserve_failure_without_replay(self):
        for asynchronous in (False, True):
            for operation in ('kill', 'send_stdin'):
                tape = UnaryTranscript(asynchronous, operation, 'zero')
                tape.starts[0] = (4321, [])
                with self.assertRaises(Exception):
                    await probe.command_case(tape.sandbox, operation, 'zero', 0)
                self.assertEqual(len(tape.signals()), 1)
                self.assertEqual(len(tape.closed), 2)
            tape = UnaryTranscript(asynchronous, 'kill', 'zero')
            tape.starts[0] = wire.deadline()
            with self.assertRaises(Exception):
                await probe.command_case(tape.sandbox, 'kill', 'zero', 0)
            self.assertEqual([item[0] for item in tape.calls], ['start'])

    async def test_primary_and_both_cleanup_failures_survive_and_health_does_not_mask_them(self):
        for asynchronous in (False, True):
            tape = UnaryTranscript(asynchronous, 'send_stdin', 'zero')
            tape.starts[1] = (5000, wire.end('', code=17, error='exit status 17'))
            tape.cleanup_error = RuntimeError('signal reply unknown')
            close_error = RuntimeError('close reply unknown')
            module = wire.async_command if asynchronous else wire.sync_command
            cls = module.AsyncCommandHandle if asynchronous else module.CommandHandle
            original = cls.disconnect
            async def disconnect(handle):
                await wire.inputs.call(original, handle)
                raise close_error
            with patch.object(cls, 'disconnect', new=disconnect), self.assertRaises(BaseExceptionGroup) as caught:
                await probe.command_case(tape.sandbox, 'send_stdin', 'zero', 0)
            self.assertIsInstance(caught.exception.exceptions[0], CommandExitException)
            self.assertEqual(list(caught.exception.exceptions[1:]), [tape.cleanup_error, close_error])
            self.assertEqual(len(tape.signals()), 1)
            self.assertEqual(len(tape.closed), 2)
            self.assertEqual([item[0] for item in tape.calls].count('start'), 2)

    async def test_cancelled_signal_and_false_acknowledgement_never_cause_a_second_signal(self):
        for asynchronous in (False, True):
            for failure in (asyncio.CancelledError(), ConnectError(Code.NOT_FOUND, 'no process')):
                tape = UnaryTranscript(asynchronous, 'kill', 'zero')
                tape.operation_error = failure
                with self.assertRaises(asyncio.CancelledError if isinstance(failure, asyncio.CancelledError) else AssertionError):
                    await probe.command_case(tape.sandbox, 'kill', 'zero', 0)
                self.assertEqual(len(tape.signals()), 1)
                self.assertEqual(len(tape.closed), 2)
