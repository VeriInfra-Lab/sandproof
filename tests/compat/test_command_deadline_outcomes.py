"""Fixed SDK deadline/terminal transcripts; no real-time race or product VM."""
from contextlib import ExitStack
import unittest

from e2b.envd.process import process_pb as pb
from protobuf import Oneof
from sdk_support import isolated_sdk_environment
import test_command_probe_cleanup as wire

probe = wire.deadlines


class DeadlineOutcomeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    async def test_run_records_exact_killed_terminal_without_claiming_rpc_deadline(self):
        for asynchronous in (False, True):
            for background in (False, True):
                for marker in (False, True):
                    with self.subTest(asynchronous=asynchronous, background=background, marker=marker):
                        tape = wire.Transcript(asynchronous,
                            [(4321, wire.end('', code=-1, error='signal: killed'))])
                        tape.sandbox.files.exists.return_value = marker
                        row = await probe.run_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1, background)
                        self.assertEqual(row['outcome'], 'command_terminated')
                        self.assertEqual(row['expected_outcomes'], ['stream_deadline', 'command_terminated'])
                        self.assertTrue(row['terminal_observed'])
                        self.assertEqual(row['original_pid'], 4321 if background else None)
                        self.assertIsNone(row['unacknowledged_candidates'])
                        self.assertEqual(row['observed_start_markers'], int(marker))
                        self.assertEqual(row['terminal_result'], dict(exit_code=-1, stdout='', stderr='', error='signal: killed'))
                        self.assertEqual([item[0] for item in tape.calls], ['start'])
                        self.assertEqual(tape.calls[0][2]['timeout_ms'], 1)
                        self.assertTrue(tape.calls[0][1].stdin)
                        self.assertEqual(len(tape.closed), 1)

    async def test_run_keeps_stream_failure_distinct_even_when_sdk_had_seen_end_before_error(self):
        for asynchronous in (False, True):
            for background in (False, True):
                for prefix in ([], wire.end('', code=-1, error='signal: killed')):
                    with self.subTest(asynchronous=asynchronous, background=background, prefix=bool(prefix)):
                        tape = wire.Transcript(asynchronous, [(4321, prefix + [wire.deadline()])], lists=[[]])
                        row = await probe.run_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1, background)
                        self.assertEqual(row['outcome'], 'stream_deadline')
                        self.assertFalse(row['terminal_observed'])
                        self.assertIsNone(row['terminal_result'])
                        self.assertEqual(tape.signals(), [Oneof('pid', 4321)] if background else [])
                        self.assertEqual(len(tape.closed), 1)

    async def test_wrong_terminal_fields_and_unexpected_success_are_still_failures(self):
        cases = [wire.end('', code=code, error='signal: killed') for code in (-9, 17, 137)]
        cases += [wire.end('', code=-1, error=error) for error in (None, 'signal: terminated', 'killed')]
        cases += [wire.end('unexpected', code=-1, error='signal: killed'), wire.end(),
            [('data', pb.ProcessEvent.DataEvent(output=Oneof('stderr', b'unexpected')))] +
            wire.end('', code=-1, error='signal: killed')]
        for asynchronous in (False, True):
            for terminal in cases:
                with self.subTest(asynchronous=asynchronous, terminal=terminal):
                    tape = wire.Transcript(asynchronous, [(4321, terminal)])
                    with self.assertRaises(AssertionError):
                        await probe.run_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1, True)
                    self.assertEqual(tape.signals(), [])
                    self.assertEqual(len(tape.closed), 1)

    async def test_connect_stream_expiry_does_not_accept_command_death(self):
        for asynchronous in (False, True):
            tape = wire.Transcript(asynchronous, [(4321, None)],
                [(4321, wire.end('', code=-1, error='signal: killed'))])
            with self.assertRaises(AssertionError):
                await probe.connect_case(tape.sandbox, asynchronous, 'timeout', 'negative', -1)
            self.assertEqual(tape.signals(), [])
            self.assertEqual([item[0] for item in tape.calls], ['start', 'connect'])
            self.assertEqual(tape.calls[0][2]['timeout_ms'], 60000)
            self.assertEqual(tape.calls[1][2]['timeout_ms'], 1)
            self.assertEqual(len(tape.closed), 2)

    async def test_no_execution_deadline_does_not_accept_killed_end_as_expected_timeout(self):
        for asynchronous in (False, True):
            for field, case, value in (('timeout', 'zero', 0), ('request_timeout', 'negative', -1)):
                with self.subTest(asynchronous=asynchronous, field=field):
                    tape = wire.Transcript(asynchronous, [(4321, wire.end('', code=-1, error='signal: killed'))])
                    if asynchronous and field == 'request_timeout':
                        tape.lists.append([])
                        tape.sandbox.files.exists.return_value = False
                        row = await probe.run_case(tape.sandbox, asynchronous, field, case, value, True)
                        self.assertEqual(row['outcome'], 'opening_deadline')
                        self.assertFalse(row['terminal_observed'])
                        self.assertIsNone(row['terminal_result'])
                        self.assertEqual(tape.signals(), [])
                        continue
                    with self.assertRaises(AssertionError):
                        await probe.run_case(tape.sandbox, asynchronous, field, case, value, True)
                    self.assertEqual(tape.signals(), [])

    async def test_wait_reports_first_outcome_and_allows_deadline_before_marker(self):
        for asynchronous in (False, True):
            for first_timeout in (False, True):
                for marker in (False, True):
                    with self.subTest(asynchronous=asynchronous, first_timeout=first_timeout, marker=marker):
                        terminal = wire.end('', code=-1, error='signal: killed')
                        tape = wire.Transcript(asynchronous,
                            [(4321, [wire.deadline()] if first_timeout else terminal), (5000, wire.end('healthy'))],
                            [(4321, terminal)], [wire.original_row, []])
                        tape.sandbox.files.exists.return_value = marker
                        row = await probe.wait_deadline_case(tape.sandbox, True)
                        self.assertEqual(row['initial_wait_outcome'], 'stream_deadline' if first_timeout else 'command_terminated')
                        self.assertEqual(row['outcome'], 'command_terminated')
                        self.assertEqual(row['start_markers'], int(marker))
                        self.assertEqual(row['terminal_result'], dict(exit_code=-1, stdout='', stderr='', error='signal: killed'))
                        self.assertEqual(tape.signals(), [])
                        self.assertEqual(len(tape.closed), 3)
                        self.assertEqual([item[1].process.selector for item in tape.calls if item[0] == 'connect'],
                                         [Oneof('pid', 4321)])
                        self.assertEqual([item[2]['timeout_ms'] for item in tape.calls if item[0] == 'start'], [1000, 15000])
                        self.assertEqual(tape.calls[-1][1].process.args[-1], 'printf healthy')

    async def test_connect_wait_still_requires_completed_original_marker(self):
        for asynchronous in (False, True):
            tape = wire.Transcript(asynchronous, [(4321, None)],
                [(4321, [wire.deadline()]), (4321, wire.end())], [wire.original_row])
            tape.sandbox.files.exists.return_value = False
            with self.assertRaises(AssertionError):
                await probe.wait_deadline_case(tape.sandbox, False)
            self.assertEqual(tape.signals(), [])
            self.assertEqual([item[0] for item in tape.calls].count('start'), 1)
            self.assertEqual(len(tape.closed), 3)
