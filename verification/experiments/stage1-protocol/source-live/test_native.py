"""Verifier regressions only: synthetic harness logs are never native evidence."""
import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest import mock

import native
import support

ROOT = None


class NativeVerifierTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.project = Path(self.temp.name)
        self.source = self.project / support.SOURCE
        self.source.parent.mkdir(parents=True)
        self.original = (ROOT / support.SOURCE).read_text()
        self.source.write_text(self.original + '\n' + (native.HERE / 'native-tests.rs.in').read_text())
        self.log = self.project / 'synthetic-tool-regression.log'

    def line(self, name):
        lines = self.source.read_text().splitlines()
        begin = next(i for i, s in enumerate(lines) if re.match(r'    fn ' + name + r'\(', s))
        end = next((i for i in range(begin+1, len(lines)) if re.match(r'    fn ', lines[i])), len(lines))
        locations = [i+1 for i in range(begin+1, end) if lines[i].lstrip().startswith('assert!(')]
        negative = [n for n in locations if lines[n-1].lstrip().startswith('assert!(!live_native(')]
        return (negative or locations)[0]

    def fixture(self, failures=(), *, skip=None, extra=None, duplicate=False, wrong_file=None,
                wrong_line=None, wrong_message=None, summary=None, panic=True):
        tests = sorted(native.test_names())
        prefix = 'controller::lifecycle_inbox::source_work::snapshot::proof_source_live_contract_tests::'
        failed = set(failures)
        statuses = {n: 'FAILED' if n in failed else 'ok' for n in tests}
        if skip:
            statuses[skip] = 'ignored'
        rows = ['running 11 tests'] + [f'test {prefix}{n} ... {statuses[n]}' for n in tests]
        if duplicate:
            rows.append(rows[-1])
        if extra:
            rows.append(f'test {prefix}{extra} ... ok')
        if failed:
            rows += ['', 'failures:', '']
            for name in sorted(failed):
                rows += [f'---- {prefix}{name} stdout ----']
                if panic:
                    rows += [f"thread '{prefix}{name}' panicked at {wrong_file or support.SOURCE}:{wrong_line or self.line(name)}:9:",
                             wrong_message or 'assertion failed: !live_native(&native, local, 3, 4, 20)', '']
            rows += ['failures:', *[prefix + n for n in sorted(failed)]]
        rows += ['', summary or f'test result: {"FAILED" if failed else "ok"}. {11-len(failed)} passed; {len(failed)} failed; 0 ignored; 0 measured; 0 filtered out;']
        self.log.write_text('\n'.join(rows) + '\n')
        return self.log

    def assert_rejected(self, expected, code=101, **kwargs):
        self.fixture(**kwargs)
        with self.assertRaises(ValueError):
            native.assertions(self.log, set(expected), self.project, code)

    def test_current_source_inventory_admission(self):
        head, _ = support.source_guard(ROOT)
        self.assertEqual(len(head), 40)

    def test_three_actual_mutations_and_inverse(self):
        for view in ('skip-version', 'allow-expired', 'ignore-startup'):
            with self.subTest(view=view):
                wrong = support.mutate(self.original, view)
                self.assertNotEqual(wrong, self.original)
                start = self.original.index('fn live_native(')
                end = self.original.index('\npub(super) fn current<', start)
                self.assertTrue(wrong.startswith(self.original[:start]))
                self.assertTrue(wrong.endswith(self.original[end:]))
                self.assertEqual(support.restore(wrong, view), self.original)

    def test_unrelated_source_change_rejected(self):
        with self.assertRaisesRegex(ValueError, 'original Rust body changed'):
            support.mutate(self.original + '\n', 'skip-version')

    def test_unknown_view_rejected(self):
        with self.assertRaisesRegex(ValueError, 'unknown'):
            support.mutate(self.original, 'different-contract')

    def test_changed_wrong_source_cannot_be_restored(self):
        wrong = support.mutate(self.original, 'skip-version')
        with self.assertRaises(ValueError):
            support.restore(wrong + '\n', 'skip-version')

    def test_normal_synthetic_harness_log(self):
        self.fixture()
        result = native.assertions(self.log, set(), self.project, 0)
        self.assertFalse(result['property_rejected'])

    def test_three_same_property_rejection_log_formats(self):
        for view in ('skip-version', 'allow-expired', 'ignore-startup'):
            with self.subTest(view=view):
                failures = native.FAILURES[view]
                self.fixture(failures)
                result = native.assertions(self.log, failures, self.project, 101)
                self.assertEqual(set(result['assertion_sites']), failures)

    def test_compiler_error_is_not_property_rejection(self):
        self.log.write_text('error: unresolved import\n')
        with self.assertRaises(ValueError):
            native.assertions(self.log, {'proof_source_live_version'}, self.project, 101)

    def test_missing_native_summary_rejected(self):
        self.fixture()
        self.log.write_text(self.log.read_text().split('\ntest result:', 1)[0])
        with self.assertRaises(ValueError):
            native.assertions(self.log, set(), self.project, 0)

    def test_extra_test_rejected(self):
        self.assert_rejected([], code=0, extra='proof_source_live_unregistered')

    def test_duplicate_test_rejected(self):
        self.assert_rejected([], code=0, duplicate=True)

    def test_skipped_test_rejected(self):
        self.assert_rejected([], code=0, skip='proof_source_live_normal')

    def test_wrong_failure_set_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_deadline'])

    def test_exit_zero_cannot_certify_rejection(self):
        self.assert_rejected(['proof_source_live_version'], code=0, failures=['proof_source_live_version'])

    def test_wrong_panic_file_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'],
                             wrong_file='controller-service/src/lib.rs')

    def test_wrong_test_owner_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'],
                             wrong_line=self.line('proof_source_live_normal'))

    def test_setup_failure_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'],
                             wrong_line=self.line('proof_source_live_version')-1)

    def test_oom_panic_not_property_rejection(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'],
                             wrong_message='allocation failure')

    def test_missing_panic_location_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'], panic=False)

    def test_wrong_summary_count_rejected(self):
        self.assert_rejected(['proof_source_live_version'], failures=['proof_source_live_version'],
                             summary='test result: FAILED. 9 passed; 2 failed; 0 ignored;')

    def test_formal_role_cannot_start_native_commands(self):
        with mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'formal'}), \
                mock.patch.object(support, 'module', side_effect=AssertionError('provider reached')), \
                mock.patch.object(native.subprocess, 'run', side_effect=AssertionError('native command reached')):
            result = native.run(ROOT, self.project / 'rejected-native-role')
        self.assertFalse(result['complete'])
        self.assertEqual(result['commands'], [])
        self.assertIn('native execution belongs', result['error'])
        self.assertFalse(result['ning_acceptance'])

    def test_check_only_never_claims_native_pass(self):
        result = native.check_only(ROOT)
        self.assertEqual(len(result['views']), 5)
        self.assertTrue(result['complete_source_preparation'])
        self.assertFalse(result['native_assertions_executed'])
        self.assertFalse(result['ning_acceptance'])
        self.assertFalse(result['full_v3'])
        self.assertTrue(all(not x['compiled'] for x in result['views'].values()))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    args, rest = parser.parse_known_args()
    ROOT = args.repo.resolve()
    unittest.main(argv=[sys.argv[0], *rest])
