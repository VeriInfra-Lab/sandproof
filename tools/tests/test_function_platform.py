import subprocess
import os
import tempfile
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

from tools import check_function_tests as entry


class SelectionTests(unittest.TestCase):
    def case(self, identity):
        result = unittest.FunctionTestCase(lambda: None)
        result.id = lambda: identity
        return result

    def test_linux_executes_all_cases_and_portable_reports_only_explicit_groups(self):
        ids = ['test_docs.DocTests.test_links',
               'test_template_build_windows.WindowBridgeTests.test_boundary',
               'test_new_module.Tests.test_new_case']
        for mode, expected in (('linux', ids), ('portable', [ids[0], ids[2]])):
            suite = unittest.TestSuite([self.case(name) for name in ids])
            selected, excluded = entry.partition(suite, mode)
            self.assertEqual([test.id() for test in selected], expected)
            self.assertEqual(sum(excluded.values()), 0 if mode == 'linux' else 1)

    def test_full_mode_requires_linux_and_supported_interpreter(self):
        for version in ((3, 9), (3, 11), (3, 14)):
            with patch.object(entry.sys, 'version_info', version), self.assertRaisesRegex(ValueError, 'Python'):
                entry.preflight('portable')
        for version in ((3, 12), (3, 13)):
            with patch.object(entry.sys, 'version_info', version), patch.object(entry.sys, 'platform', 'darwin'):
                entry.preflight('portable')
                with self.assertRaisesRegex(ValueError, 'require Linux'):
                    entry.preflight('linux')

    def test_declared_groups_exist_in_the_real_suite(self):
        suite = unittest.TestLoader().discover(str(entry.ROOT / 'tools/tests'))
        groups = {'.'.join(test.id().split('.')[:2]) for test in entry.leaves(suite)}
        self.assertEqual(set(entry.LINUX_GROUPS) - groups, set())

    def test_explicit_supported_interpreter_is_respected(self):
        root = Path(__file__).resolve().parents[2]
        result = subprocess.run(['bash', str(root / 'scripts/python-entry.sh'), '-c', 'print("chosen")'],
            env=os.environ | {'SANDPROOF_PYTHON': sys.executable},
            capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, 'chosen\n')

    def test_interpreter_rejection_stops_before_any_cargo_action(self):
        root = Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            rejected = stage / 'rejected-python'
            rejected.write_text('#!/bin/sh\necho "Python version rejected" >&2\nexit 17\n')
            rejected.chmod(0o700)
            cargo = stage / 'cargo'
            cargo.write_text('#!/bin/sh\ntouch "' + str(stage / 'cargo-ran') + '"\nexit 91\n')
            cargo.chmod(0o700)
            result = subprocess.run(['bash', str(root / 'scripts/check-function.sh')],
                env=os.environ | {'SANDPROOF_PYTHON': str(rejected),
                                  'PATH': str(stage) + os.pathsep + os.environ['PATH']},
                capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 17, result.stderr)
            self.assertFalse((stage / 'cargo-ran').exists())
            self.assertNotIn('Functional checks passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
