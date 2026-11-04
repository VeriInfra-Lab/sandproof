"""Capture-admission regressions. Altered IR fixtures are not Rust counterexamples."""
import argparse
import copy
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import capture
import support

ROOT = FIXTURE = None


class CaptureAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        assert support.sha(FIXTURE) == 'd2886b5258bed0282a088ea768a277c60805fc0f2375d5f894ce6829cf50da8d'
        cls.raw = json.loads(FIXTURE.read_text())

    def changed(self):
        return copy.deepcopy(self.raw)

    def test_retained_actual_IR_fixture_is_transparent(self):
        result = capture.validate_ir(self.raw, 'x86_64-unknown-linux-gnu')
        self.assertEqual(result['function_count'], 27)
        self.assertEqual(result['owned_transparent_functions'], 7)

    def test_missing_or_failed_translation_rejected(self):
        for missing in (True, False):
            raw = self.changed()
            if missing:
                del raw['has_errors']
            else:
                raw['has_errors'] = True
            with self.subTest(missing=missing), self.assertRaises(ValueError):
                capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_target_mismatch_rejected(self):
        with self.assertRaises(ValueError):
            capture.validate_ir(self.raw, 'aarch64-unknown-linux-gnu')

    def test_alternate_root_rejected(self):
        raw = self.changed()
        raw['translated']['options']['start_from'] = ['crate::a_different_function']
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_removed_owned_include_rejected(self):
        raw = self.changed()
        raw['translated']['options']['include'].remove('sandbox_controller_core')
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_added_business_opaque_rejected(self):
        raw = self.changed()
        raw['translated']['options']['opaque'].append(capture.ROOTS[0])
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_borrow_or_typecheck_bypasses_rejected(self):
        for flag in ('skip_borrowck', 'no_typecheck', 'monomorphize', 'remove_unused_clauses'):
            for value in (True, None):
                raw = self.changed()
                raw['translated']['options'][flag] = value
                with self.subTest(flag=flag, value=value), self.assertRaises(ValueError):
                    capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_exclusion_rejected(self):
        raw = self.changed()
        raw['translated']['options']['exclude'] = [capture.ROOTS[0]]
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_missing_real_root_rejected(self):
        raw = self.changed()
        raw['translated']['fun_decls'] = [f for f in raw['translated']['fun_decls']
            if not f or f['item_meta']['name'] != capture.ROOT_NAME]
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_duplicate_real_root_rejected(self):
        raw = self.changed()
        root = next(f for f in raw['translated']['fun_decls'] if f and f['item_meta']['name'] == capture.ROOT_NAME)
        raw['translated']['fun_decls'].append(copy.deepcopy(root))
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_wrong_real_root_arity_rejected(self):
        raw = self.changed()
        root = next(f for f in raw['translated']['fun_decls'] if f and f['item_meta']['name'] == capture.ROOT_NAME)
        root['signature']['inputs'].pop()
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_hidden_V4_body_rejected(self):
        raw = self.changed()
        guard = next(f for f in raw['translated']['fun_decls'] if f and f['def_id'] == 20)
        guard['item_meta']['opacity'] = 'Opaque'
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_erased_owned_body_rejected(self):
        raw = self.changed()
        guard = next(f for f in raw['translated']['fun_decls'] if f and f['def_id'] == 20)
        guard['body'] = 'Opaque'
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_missing_actual_callback_rejected(self):
        raw = self.changed()
        raw['translated']['fun_decls'] = [f for f in raw['translated']['fun_decls'] if not f or f['def_id'] != 23]
        with self.assertRaises(ValueError):
            capture.validate_ir(raw, 'x86_64-unknown-linux-gnu')

    def test_low_capacity_rejected_before_copy(self):
        with self.assertRaises(ValueError):
            capture.resource_admission(capture.FLOOR + capture.WRITE_BUDGET - 1)
        capture.resource_admission(capture.FLOOR + capture.WRITE_BUDGET)
        self.assertEqual(capture.FLOOR, 64 * 2**30)

    def test_native_role_cannot_start_capture(self):
        with tempfile.TemporaryDirectory() as parent, \
                mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'native'}), \
                mock.patch.object(support, 'module', side_effect=AssertionError('provider reached')), \
                mock.patch.object(capture.subprocess, 'Popen', side_effect=AssertionError('capture reached')):
            result = capture.run(ROOT, Path(parent) / 'rejected-capture', Path(parent) / 'unused-lock')
        self.assertFalse(result['capture_complete'])
        self.assertFalse(result['formal_acceptance'])
        self.assertFalse(result['complete'])
        self.assertEqual(result['commands'], [])

    def test_real_runner_checks_capacity_before_providers(self):
        with tempfile.TemporaryDirectory() as parent, \
                mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'formal'}), \
                mock.patch.object(capture.os, 'sched_getaffinity', return_value={0}), \
                mock.patch.object(capture.shutil, 'disk_usage', return_value=SimpleNamespace(free=capture.FLOOR)), \
                mock.patch.object(support, 'module', side_effect=AssertionError('provider reached')), \
                mock.patch.object(capture.subprocess, 'Popen', side_effect=AssertionError('capture reached')):
            lock = Path(parent) / 'local-regression-lock'
            lock.touch()
            result = capture.run(ROOT, Path(parent) / 'low-space-result', lock)
        self.assertFalse(result['capture_complete'])
        self.assertFalse(result['complete'])
        self.assertEqual(result['commands'], [])
        self.assertIn('64GiB floor', result['error'])

    def test_inherited_descriptor_must_be_same_lock(self):
        with tempfile.TemporaryDirectory() as parent, \
                mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'formal'}), \
                mock.patch.object(capture.os, 'sched_getaffinity', return_value={0}), \
                mock.patch.object(support, 'module', side_effect=AssertionError('provider reached')):
            requested = Path(parent) / 'requested-local-lock'
            requested.touch()
            with (Path(parent) / 'different-local-lock').open('w+') as other:
                result = capture.run(ROOT, Path(parent) / 'wrong-descriptor-result', requested, other.fileno())
        self.assertFalse(result['capture_complete'])
        self.assertEqual(result['commands'], [])
        self.assertIn('another lock', result['error'])

    def test_input_admission_not_formal_completion(self):
        result = capture.input_admission(ROOT)
        self.assertEqual(tuple(result['views']), capture.VIEWS)
        self.assertTrue(result['input_admission_complete'])
        self.assertFalse(result['capture_complete'])
        self.assertFalse(result['complete'])
        self.assertFalse(result['formal_acceptance'])
        self.assertFalse(result['formal_counterexamples_reached'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--retained-IR-fixture', type=Path, required=True)
    args, rest = parser.parse_known_args()
    ROOT, FIXTURE = args.repo.resolve(), args.retained_IR_fixture.resolve()
    unittest.main(argv=[sys.argv[0], *rest])
