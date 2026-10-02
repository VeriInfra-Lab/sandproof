"""Phase diagnostics share tool-log limits and cannot replace worker failures."""
import io
import json
import unittest
from unittest.mock import patch

from tools.template_inputs import InvalidTemplate
from tools.template_worker import Commands, LogBudget


class PhaseLogTests(unittest.TestCase):
    def test_flushes_start_before_work_and_records_monotonic_duration(self):
        log = io.BytesIO()
        now = [100.0]
        with patch('tools.template_worker.time.monotonic', side_effect=lambda: now[0]):
            runner = Commands(120, log, 4096)
            now[0] = 102.0
            with runner.phase('rootfs_unpack'):
                start = self.events(log)[0]
                self.assertEqual(start, dict(phase='rootfs_unpack', state='started',
                                             started_seconds=2.0, elapsed_seconds=0.0))
                now[0] = 109.5
            self.assertEqual(self.events(log)[1], dict(phase='rootfs_unpack', state='completed',
                                                      started_seconds=2.0, elapsed_seconds=7.5))
            self.assertEqual(runner.deadline, 120)
        self.assertEqual(runner.budget.remaining, 4096 - len(log.getvalue()))

    def test_failed_phase_does_not_claim_completion_or_log_exception_contents(self):
        log = io.BytesIO()
        failure = TimeoutError('private detail must not enter diagnostics')
        with self.assertRaises(TimeoutError) as result:
            with Commands(120, log, 4096).phase('oci_validate'):
                raise failure
        self.assertIs(result.exception, failure)
        self.assertEqual([e['state'] for e in self.events(log)], ['started', 'failed'])
        self.assertNotIn(b'private detail', log.getvalue())

    def test_failed_phase_preserves_cause_when_diagnostics_cannot_be_written(self):
        for break_log in (lambda runner: setattr(runner.budget, 'remaining', 0),
                          lambda runner: runner.log.close()):
            with self.subTest(break_log=break_log):
                runner = Commands(120, io.BytesIO(), 4096)
                failure = OSError('original worker failure')
                with self.assertRaises(OSError) as result:
                    with runner.phase('ext4_create'):
                        break_log(runner)
                        raise failure
                self.assertIs(result.exception, failure)

    def test_diagnostics_share_daemon_budget_and_cannot_exceed_limit(self):
        budget = LogBudget(30)
        budget.take(23)  # Bytes already consumed by the separate daemon log.
        log = io.BytesIO()
        runner = Commands(120, log, 9999, budget=budget)
        with self.assertRaisesRegex(InvalidTemplate, 'build log limit'):
            with runner.phase('acquire'):
                self.fail('work must not start after logging exceeds the budget')
        self.assertEqual(len(log.getvalue()), 7)
        self.assertEqual(budget.remaining, 0)

    def test_successful_work_still_rejects_diagnostic_log_overflow(self):
        log = io.BytesIO()
        runner = Commands(120, log, 4096)
        with self.assertRaisesRegex(InvalidTemplate, 'build log limit'):
            with runner.phase('disk_digest'):
                runner.budget.remaining = 0
        self.assertEqual([e['state'] for e in self.events(log)], ['started'])

    @staticmethod
    def events(log):
        return [json.loads(line.removeprefix('[sandproof-phase] '))
                for line in log.getvalue().decode().splitlines() if line]


if __name__ == '__main__':
    unittest.main()
