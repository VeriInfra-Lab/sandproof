"""Probe failure/cancellation must retain identities and attempt every cleanup."""
import asyncio
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
try:
    import template_resource_pressure as pressure
finally:
    sys.path.remove(str(SYSTEM))
from tools.generic_capacity import GIB, MIB, plan
from tools.template_acceptance import budgets


def deployment():
    entries = [{'name': str(i), 'requirements': {'cpus': 1, 'memory_mib': 2048,
                'disk_mib': 10240, 'build_timeout_sec': 600}} for i in range(4)]
    return budgets(plan(entries, 1, 32 * GIB, 64 * MIB), 32 * GIB, 64 * MIB)


def templates():
    return [{'template': {'templateId': 'tpl-' + str(i) * 32, 'cpuCount': 1,
                          'memoryMib': 2048, 'diskBytes': 10 * GIB}} for i in (1, 2)]


class PressureFailureTests(unittest.TestCase):
    def test_four_instance_cpu_memory_and_disk_checked_before_dispatch(self):
        shape = {'cpus': 1, 'memory_mib': 2048, 'disk_mib': 10240}
        pressure.validate_cohort([shape, shape], deployment())
        for field, value in (('cpus', 2), ('memory_mib', 8192), ('disk_mib', 20480)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                pressure.validate_cohort([shape, shape | {field: value}], deployment())

    def invoke(self, root, creates, *, install_error=None, audit=None):
        async def run():
            with patch.object(pressure.AsyncSandbox, 'create', AsyncMock(side_effect=creates)) as create, \
                 patch.object(pressure, 'clean', AsyncMock()), \
                 patch.object(pressure, 'install', AsyncMock(side_effect=install_error)):
                try:
                    await pressure.exercise(root=root, urls=['http://127.0.0.1:1'] * 3,
                        key='synthetic', builder_key='private-synthetic', rows=templates(), deployment=deployment(),
                        supervisor=Mock(), restart=Mock(), source_entry={},
                        audit_shape=lambda env: {'disk_inode': int(env.sandbox_id)},
                        completion_audit=audit or Mock(return_value={'exact_cleanup': 'passed'}))
                finally:
                    self.calls = create.await_count
        return asyncio.run(run())

    def sandbox(self, index, kill_error=None):
        return SimpleNamespace(sandbox_id=str(index), kill=AsyncMock(return_value=True, side_effect=kill_error))

    def test_lost_create_reply_does_not_retry_and_cleans_known_instance(self):
        first = self.sandbox(1)
        audit = Mock(return_value={'exact_cleanup': 'passed'})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(TimeoutError):
                self.invoke(root, [first, TimeoutError('lost reply')], audit=audit)
            report = json.loads((root / 'resource-pressure.json').read_text())
        self.assertEqual(self.calls, 2)
        first.kill.assert_awaited_once()
        audit.assert_called_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['error_type'], 'TimeoutError')
        self.assertEqual(report['identities'][0]['sandbox_id'], '1')

    def test_cancelled_probe_still_attempts_all_four_original_stops(self):
        cohort = [self.sandbox(i) for i in range(1, 5)]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(asyncio.CancelledError):
                self.invoke(root, cohort, install_error=asyncio.CancelledError())
            report = json.loads((root / 'resource-pressure.json').read_text())
        for sandbox in cohort:
            sandbox.kill.assert_awaited_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['error_type'], 'CancelledError')
        self.assertEqual(len(report['identities']), 4)

    def test_one_stop_error_does_not_skip_other_stops_or_external_audit(self):
        cohort = [self.sandbox(i, TimeoutError() if i == 4 else None) for i in range(1, 5)]
        audit = Mock(return_value={'exact_cleanup': 'passed'})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(BaseExceptionGroup):
                self.invoke(root, cohort, install_error=ValueError('package failure'), audit=audit)
            report = json.loads((root / 'resource-pressure.json').read_text())
        for sandbox in cohort:
            sandbox.kill.assert_awaited_once()
        audit.assert_called_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['exact_cleanup'], 'passed')
        self.assertEqual(report['error_type'], 'ValueError')
        self.assertEqual(report['cleanup_error_types'], ['TimeoutError'])


if __name__ == '__main__':
    unittest.main()
