"""The privileged probe must refuse ordinary CI before performing host actions."""
from pathlib import Path
import copy
import json
import runpy
import subprocess
import sys
import unittest


class DaemonRestartGuard(unittest.TestCase):
    def test_direct_invocation_refuses_before_reading_candidate_or_configuring_network(self):
        probe = Path(__file__).resolve().parents[1] / 'system/guest_daemon_restart.py'
        for optimize in ([], ['-O']):
            with self.subTest(optimize=optimize):
                result = subprocess.run([sys.executable, *optimize, str(probe),
                    '--service', '/nonexistent/service', '--worker', '/nonexistent/worker',
                    '--configuration', '/nonexistent/configuration'],
                    capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
                self.assertNotIn('FileNotFoundError', result.stderr)
                if not optimize:
                    self.assertIn('requires root as PID 1 in a new PID namespace', result.stderr)
                else:
                    self.assertIn('assertions', result.stderr)


class TaskReportValidation(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        probe = Path(__file__).resolve().parents[1] / 'system/guest_daemon_restart.py'
        cls.module = runpy.run_path(str(probe))

    def fixture(self):
        value = dict(schema_version=1, scope='process_service_owned_tasks', exact=True,
                     total=dict(live=0, peak=2))
        for group, names in self.module['TASK_ROLES'].items():
            value[group] = dict(count=dict(live=0, peak=1), roles=[
                dict(name=name, count=dict(live=0, peak=int(index == 0)))
                for index, name in enumerate(names)])
        return value

    def parse(self, value, prefix=b''):
        return self.module['task_report'](
            prefix + b'task accounting: ' + json.dumps(value).encode() + b'\n', prefix)

    def test_accepts_settled_coherent_report_with_exact_optional_prefix(self):
        value = self.fixture()
        for prefix in (b'', b'new VM state initialized\n'):
            self.assertEqual(self.parse(value, prefix), value)

    def test_refuses_inexact_unsettled_or_inconsistent_counters(self):
        cases = [
            ('scope', 'threads'), ('exact', False), ('exact', 1), ('schema_version', True),
            ('schema_version', 2), ('total', dict(live=1, peak=2)),
            ('total', dict(live=0, peak=True)), ('total', dict(live=0, peak=3)),
            ('total', dict(live=0, peak=0)), ('total', dict(live=0, peak=-1)),
            ('total', dict(live=0, peak=1 << 64)), ('extra', 'forbidden'),
        ]
        for key, replacement in cases:
            value = self.fixture()
            value[key] = replacement
            with self.subTest(key=key, replacement=replacement), self.assertRaises(AssertionError):
                self.parse(value)
        for mutation in (
            lambda group: group['roles'].pop(),
            lambda group: group['roles'].reverse(),
            lambda group: group['roles'][0].update(name='unknown'),
            lambda group: group['roles'][0]['count'].update(live=1),
            lambda group: group['roles'][0]['count'].update(peak=2),
            lambda group: group['count'].update(peak=2),
            lambda group: group.update(extra=1),
        ):
            value = copy.deepcopy(self.fixture())
            mutation(value['blocking'])
            with self.assertRaises(AssertionError):
                self.parse(value)

    def test_refuses_missing_duplicate_trailing_or_oversized_output(self):
        report = b'task accounting: ' + json.dumps(self.fixture()).encode() + b'\n'
        for output in (b'', report[:-1], report + b'\n', b'wrong\n' + report,
                       report.replace(b'"exact": true', b'"exact": true, "exact": true'),
                       b'task accounting: ' + b' ' * 4096 + b'\n'):
            with self.subTest(output=output[:80]), self.assertRaises((AssertionError, ValueError)):
                self.module['task_report'](output)
