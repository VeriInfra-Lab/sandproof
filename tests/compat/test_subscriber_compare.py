"""Actual fixed SDK handles with controlled subscription transcripts; no VM."""
import asyncio
import copy
from contextlib import ExitStack
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_support import isolated_sdk_environment
import test_background_compare as original

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_guest_compare as paired
import sdk_subscriber_compare as subscribers


def failures(error):
    if isinstance(error, BaseExceptionGroup):
        return [item for child in error.exceptions for item in failures(child)]
    return [error]


class OriginalSdkSubscriberTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())

    def fixture(self, asynchronous, mode='detach-start', pid=4321):
        code = 'direct' if mode == 'detach-start' else 'reconnect-exit'
        return original.OriginalSdkTests.fixture(self, asynchronous, code, pid)

    async def run_case(self, observed, mode='detach-start'):
        return await subscribers.subscriber_case(observed, 'ubuntu', '/tmp/shared/ubuntu', mode)

    async def test_three_live_subscriptions_detach_one_and_compare_both_original_sdks(self):
        for mode in subscribers.MODES:
            rows = []
            for asynchronous in (False, True):
                observed, backend = self.fixture(asynchronous, mode, 4321 + len(rows))
                row = await self.run_case(observed, mode)
                rows.append(row)
                self.assertEqual([name for name, *_ in backend.calls],
                                 ['start', 'connect', 'connect', 'input', 'input', 'close-stdin', 'connect'])
                self.assertEqual(bytes(backend.data), subscribers.TEXT.encode() + subscribers.BINARY)
                self.assertEqual(len(backend.closed), 4)
                waits = [item['subscription'] for item in row['calls'] if item['method'] == 'handle.wait']
                self.assertEqual(waits, ['peer-a', 'peer-b', 'late'] if mode == 'detach-start'
                                 else ['start', 'peer-b', 'late'])
                detached = [item for item in row['subscriptions'] if item['name'] == row['detached']][0]
                self.assertEqual(detached['callbacks'], {'stdout': [], 'stderr': []})
                closes = [item['subscription'] for item in row['calls'] if item['method'] == 'handle.disconnect']
                self.assertEqual(len(closes), len(set(closes)))
            paired.compare([rows[0]], [rows[1]], 'actual sync/async SDK subscribers')
            self.assertNotEqual(rows[0]['pid'], rows[1]['pid'], 'comparison must preserve raw distinct PIDs')

    async def test_corruption_missing_end_and_wrong_pid_fail_with_original_ownership(self):
        for asynchronous in (False, True):
            for fault, signals, closes in [('stdout', 0, 3), ('missing-end', 1, 3),
                                           ('history', 0, 4), ('wrong-pid', 1, 2)]:
                with self.subTest(asynchronous=asynchronous, fault=fault):
                    observed, backend = self.fixture(asynchronous)
                    if fault == 'wrong-pid':
                        backend.wrong_pid = True
                    else:
                        backend.corruption = fault
                    with self.assertRaises(Exception):
                        await self.run_case(observed)
                    self.assertEqual(sum(name == 'kill' for name, *_ in backend.calls), signals)
                    self.assertEqual(sum(name == 'start' for name, *_ in backend.calls), 1)
                    self.assertEqual(len(backend.closed), closes)
                    self.assertEqual(observed.rows[-1]['status'], 'incomplete')

    async def test_unexpected_terminal_category_is_still_end_before_cleanup(self):
        for asynchronous in (False, True):
            for mode in subscribers.MODES:
                observed, backend = self.fixture(asynchronous, mode)
                backend.code = 17 if mode == 'detach-start' else 0
                backend.error = 'exit status 17' if backend.code else None
                expected = CommandExitException if backend.code else AssertionError
                with self.assertRaises(expected):
                    await self.run_case(observed, mode)
                self.assertFalse(any(name == 'kill' for name, *_ in backend.calls))
                self.assertEqual(len(backend.closed), 3)

    async def test_invalid_start_pid_is_closed_without_control_or_connect(self):
        for asynchronous in (False, True):
            observed, backend = self.fixture(asynchronous, pid=0)
            with self.assertRaises(AssertionError):
                await self.run_case(observed)
            self.assertEqual([name for name, *_ in backend.calls], ['start'])
            self.assertEqual(len(backend.closed), 1)

    async def test_unknown_start_and_second_connect_cancellation_never_replay(self):
        for asynchronous in (False, True):
            for stage in ('start', 'second-connect'):
                for error in (RuntimeError('unknown subscription reply'), asyncio.CancelledError('cancelled')):
                    with self.subTest(asynchronous=asynchronous, stage=stage, error=type(error).__name__):
                        observed, backend = self.fixture(asynchronous)
                        rpc = observed.sandbox.commands._rpc
                        name = 'start' if stage == 'start' else 'connect'
                        method, attempts = getattr(rpc, name), []
                        def fail(*args, **kwargs):
                            attempts.append(None)
                            if stage == 'start' or len(attempts) == 2:
                                raise error
                            return method(*args, **kwargs)
                        with patch.object(rpc, name, fail), self.assertRaises(type(error)) as raised:
                            await self.run_case(observed)
                        self.assertIs(raised.exception, error)
                        self.assertEqual(len(attempts), 1 if stage == 'start' else 2)
                        self.assertEqual([item[0] for item in backend.calls],
                                         [] if stage == 'start' else ['start', 'connect', 'kill'])
                        self.assertEqual(len(backend.closed), 0 if stage == 'start' else 2)
                        self.assertEqual(observed.rows[-1]['status'], 'incomplete')

    async def test_missing_end_signal_and_every_close_failure_are_retained_once(self):
        disconnect = subscribers.RecordedHandle.disconnect
        async def failed_close(handle):
            await disconnect(handle)
            # The explicitly detached stream must finish closing so that the
            # intended missing-End path, rather than an earlier failure, runs.
            if handle.name != 'start':
                raise RuntimeError('close ' + handle.name)
        for asynchronous in (False, True):
            observed, backend = self.fixture(asynchronous)
            backend.corruption = 'missing-end'
            backend.signal_error = OSError('unknown signal reply')
            with patch.object(subscribers.RecordedHandle, 'disconnect', failed_close):
                with self.assertRaises(BaseExceptionGroup) as raised:
                    await self.run_case(observed)
            messages = [str(error) for error in failures(raised.exception)]
            self.assertCountEqual(messages, ['Command ended without an end event',
                                            'unknown signal reply', 'close peer-a', 'close peer-b'])
            self.assertEqual(sum(name == 'kill' for name, *_ in backend.calls), 1)
            self.assertEqual(len(backend.closed), 3)

    async def test_comparison_rejects_wrong_identity_partial_evidence_and_output_changes(self):
        observed, _ = self.fixture(False)
        row = await self.run_case(observed)
        for mutation in ('reference', 'partial', 'output', 'input', 'start-count'):
            other = copy.deepcopy(row)
            if mutation == 'reference':
                other['subscriptions'][1]['pid'] += 1
            elif mutation == 'partial':
                other['calls'][-1]['status'] = 'incomplete'
            elif mutation == 'output':
                other['subscriptions'][1]['callbacks']['stdout'].append('4321')
            elif mutation == 'input':
                next(item for item in other['calls'] if item['method'] == 'commands.send_stdin')['args'][1] += '4321'
            else:
                other['calls'].append(copy.deepcopy(next(item for item in other['calls']
                    if item['method'] == 'commands.run' and item['kwargs'].get('background'))))
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                paired.compare([row], [other], mutation)

    async def test_resource_hooks_follow_subscription_barriers_without_changing_compared_calls(self):
        for asynchronous in (False, True):
            for mode in subscribers.MODES:
                original, _ = self.fixture(asynchronous, mode)
                reference = await self.run_case(original, mode)
                observed, backend = self.fixture(asynchronous, mode)
                phases = []
                async def sample(phase):
                    phases.append(phase)
                    if phase.endswith('/subscribed'):
                        self.assertEqual([name for name, *_ in backend.calls], ['start', 'connect', 'connect'])
                        self.assertEqual(len(backend.closed), 0)
                        self.assertFalse(backend.done)
                    elif phase.endswith('/detached'):
                        self.assertEqual(len(backend.closed), 1)
                        self.assertFalse(backend.done)
                    else:
                        self.assertTrue(backend.done)
                        self.assertEqual(len(backend.closed), 4)
                        self.assertEqual([name for name, *_ in backend.calls][-1], 'connect')
                row = await subscribers.subscriber_case(observed, 'ubuntu', '/tmp/shared/ubuntu', mode,
                                                        resource_sample=sample)
                self.assertEqual(phases, [f'subscribers/ubuntu/{mode}/{phase}'
                                          for phase in subscribers.RESOURCE_PHASES])
                paired.compare([reference], [row], 'sampled protocol calls unchanged')

    async def test_resource_hook_failure_cleans_only_unfinished_command_and_retains_all_closes(self):
        for asynchronous in (False, True):
            for stage in subscribers.RESOURCE_PHASES:
                observed, backend = self.fixture(asynchronous)
                error = RuntimeError('resource observation unknown')
                async def sample(phase):
                    if phase.endswith('/' + stage):
                        raise error
                with self.assertRaises(RuntimeError) as raised:
                    await subscribers.subscriber_case(observed, 'ubuntu', '/tmp/shared/ubuntu', 'detach-start',
                                                      resource_sample=sample)
                self.assertIs(raised.exception, error)
                self.assertEqual(sum(name == 'start' for name, *_ in backend.calls), 1)
                self.assertEqual(sum(name == 'kill' for name, *_ in backend.calls), int(stage != 'terminal'))
                self.assertEqual(len(backend.closed), 4 if stage == 'terminal' else 3)
                self.assertEqual(observed.rows[-1]['status'], 'incomplete')
