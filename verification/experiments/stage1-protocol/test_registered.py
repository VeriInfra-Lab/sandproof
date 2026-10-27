"""Admission and scheduling regressions; no compiler, VM or proof is launched."""
import ast
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('stage1_registered', HERE/'registered.py')
registered = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registered)


class Registration(unittest.TestCase):
    def test_native_keys_are_the_original_formal_prerequisites(self):
        self.assertEqual(len(registered.NATIVE_CASES), 28)
        self.assertEqual(len(set(registered.NATIVE_CASES)), 28)
        self.assertEqual(registered.native_labels(), registered.NATIVE_CASES)
        for path in HERE.glob('inbox-*/formal.py'):
            calls = [node for node in ast.walk(ast.parse(path.read_text()))
                     if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                     and node.func.id == 'native_case']
            labels = {node.args[0].value for node in calls if node.args
                      and isinstance(node.args[0], ast.Constant)}
            self.assertLessEqual(labels-{'functional'}, set(registered.NATIVE_CASES))
        self.assertNotIn('stage1-inbox-record-total', registered.NATIVE_CASES)

    def test_old_v0_and_retained_component_are_not_replaced(self):
        names = ast.parse((registered.ROOT/'tools/proof_batch.py').read_text())
        modules = next(ast.literal_eval(node.value) for node in names.body
                       if isinstance(node, ast.Assign) and any(
                           isinstance(t, ast.Name) and t.id == 'MODULES' for t in node.targets))
        self.assertEqual(len(modules), 30)
        self.assertIn('stage1-protocol', modules)
        self.assertNotIn('component.py', {Path(j['driver']).name for j in registered.formal_plan(Path('/unused'))})

    def test_full_independent_positive_and_negative_schedule(self):
        jobs = registered.formal_plan(Path('/unused'))
        names = {job['name'] for job in jobs}
        self.assertEqual(len(jobs), len(names))
        self.assertLessEqual({'replay-replay', 'replay-validation', 'replay-envelope',
            'manifest-validation', 'access-identity', 'create-observation', 'create-conversion',
            'create-guard', 'envelope-writer-correct', 'envelope-writer-swallow-error',
            'serde-binding', 'serde-binding-negative', 'parser-binding', 'parser-binding-negative',
            'decode-assembly', 'decode-assembly-negative'}, names)
        for job in jobs:
            self.assertTrue((registered.ROOT/job['driver']).is_file())
            self.assertNotIn('--prepare-only', job['args'])
            self.assertNotIn('--development-native', job['args'])
        manifest = next(j for j in jobs if j['name'] == 'manifest-validation')
        self.assertEqual(len(manifest['cases']), 6)
        self.assertIn('skip-capture-binding', manifest['cases'])

    def test_total_only_subsumes_same_variant_append_formal(self):
        jobs = registered.formal_plan(Path('/unused'))
        inbox = [job for job in jobs if Path(job['driver']).parent.name.startswith('inbox-')]
        self.assertEqual(len(inbox), 37)
        self.assertFalse(any(j['name'].startswith('inbox-append-') for j in inbox))
        for variant in registered.INBOX_CASES['inbox-append']:
            job, = [j for j in inbox if j['name'] == 'inbox-record-total-'+variant]
            self.assertEqual(set(job['native']), {'stage1-inbox-record', 'stage1-inbox-append'})
        for job in inbox:
            self.assertIn('--require-native', job['args'])
        self.assertIn('stage1-inbox-append', registered.NATIVE_CASES)
        retained = next(j for j in inbox if j['name'] == 'inbox-retained-correct')
        self.assertNotIn('--fresh-native', retained['args'])

    def test_lost_native_or_variant_registration_is_refused(self):
        registered.validate_registration()
        with patch.object(registered, 'NATIVE_CASES', tuple(
                name for name in registered.NATIVE_CASES if name != 'stage1-inbox-state')):
            with self.assertRaisesRegex(ValueError, 'missing native registration'):
                registered.validate_registration()
        variants = dict(registered.INBOX_CASES, **{'inbox-state': ('correct', 'always-current')})
        with patch.object(registered, 'INBOX_CASES', variants):
            with self.assertRaisesRegex(ValueError, 'missing/extra original variant'):
                registered.validate_registration()

    def test_wrong_machine_role_fails_before_any_work(self):
        with patch.object(registered, '_shared') as shared:
            with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='formal'):
                with self.assertRaisesRegex(ValueError, 'native host'):
                    registered.run_native(registered.ROOT, Path('/unused'))
            with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='native'):
                with self.assertRaisesRegex(ValueError, 'shared native'):
                    registered.run_formal(Path('/unused'))
            shared.assert_not_called()

    def test_formal_refuses_missing_seal_or_native_case_before_driver(self):
        for failure in ('seal', 'case'):
            shared = SimpleNamespace(validate_native=lambda _: None, native_case=lambda *a: {})
            if failure == 'seal':
                shared.validate_native = lambda _: (_ for _ in ()).throw(ValueError('changed seal'))
            else:
                shared.native_case = lambda *a: (_ for _ in ()).throw(ValueError('missing native case'))
            with patch.object(registered, '_shared', return_value=shared), patch.object(registered, '_invoke') as invoke:
                with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='formal', SANDPROOF_NATIVE_EVIDENCE='/unused'):
                    with self.assertRaisesRegex(ValueError, 'seal|native case'):
                        registered.run_formal(Path('/unused'))
                invoke.assert_not_called()

    def test_native_archive_and_result_remain_sealed_original_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            evidence = base/'commands/identity/evidence'
            evidence.mkdir(parents=True)
            raw = evidence/'source.tar'
            raw.write_bytes(b'original source bytes')
            receipt = dict(key={}, files={'evidence/source.tar': registered._sha(raw)})
            with patch.object(registered, '_shared', return_value=SimpleNamespace(digest=lambda _: 'identity')):
                with patch.dict(os.environ, SANDPROOF_NATIVE_EVIDENCE=str(base)):
                    self.assertEqual(registered._original_file(receipt, 'source.tar'), raw)
                    raw.write_bytes(b'changed source bytes')
                    with self.assertRaisesRegex(ValueError, 'changed sealed'):
                        registered._original_file(receipt, 'source.tar')

    def test_original_archive_driver_cannot_use_complete_instead_of_success(self):
        def native_case(label, root, output, action):
            if label != 'stage1-protocol':
                action()
            return {}
        shared = SimpleNamespace(native_case=native_case)
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(registered, 'validate_registration'), \
                patch.object(registered, '_shared', return_value=shared), \
                patch.object(registered, '_original_file', return_value=Path('/unused/source.tar')), \
                patch.object(registered, '_invoke'), \
                patch.object(registered, '_read', return_value=dict(complete=True, success=False)), \
                patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='native', CARGO_TARGET_DIR='/unused'):
            with self.assertRaisesRegex(ValueError, 'native driver did not complete'):
                registered.run_native(registered.ROOT, Path(temporary))

    def test_bare_crate_zero_test_success_is_rejected(self):
        def native_run(label, command, root, log):
            log.write_text('test result: ok. 0 passed; 0 failed; 0 ignored;\n')
        shared = SimpleNamespace(native_run=native_run)
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(registered, '_shared', return_value=shared):
            with self.assertRaisesRegex(ValueError, 'crate test inventory'):
                registered._crate_native(registered.ROOT, Path(temporary)/'output', 'envelope-schema')

    def test_incomplete_cases_or_missing_physical_audits_cannot_pass(self):
        job = dict(name='example', args=[], cases=['correct', 'wrong'], actual_files=True)
        actual = dict(complete=True, files={'Actual.lean': 'sha'}, audits=['actual.theorem'], service_log_sha256='log')
        valid = dict(complete=True, cases={'correct': actual, 'wrong': copy.deepcopy(actual)})
        registered._check_actual(valid, job)
        for field in ('files', 'audits', 'service_log_sha256'):
            broken = copy.deepcopy(valid)
            broken['cases']['wrong'].pop(field)
            with self.assertRaisesRegex(ValueError, 'physical-file'):
                registered._check_actual(broken, job)
        broken = copy.deepcopy(valid)
        broken['cases'].pop('wrong')
        with self.assertRaisesRegex(ValueError, 'missing/extra'):
            registered._check_actual(broken, job)
        broken = copy.deepcopy(valid)
        broken['cases']['wrong']['complete'] = False
        with self.assertRaisesRegex(ValueError, 'incomplete original'):
            registered._check_actual(broken, job)

    def test_wrong_variant_and_model_only_rejection_are_rejected(self):
        job = dict(name='inbox-state-drop-operation', args=['--variant', 'drop-operation'],
                   cases=['drop-operation'], actual_files=False)
        with self.assertRaisesRegex(ValueError, 'wrong actual variant'):
            registered._check_actual(dict(complete=True, variant='correct'), job)
        job = dict(name='serde-binding-negative', args=[], cases=[], actual_files=False)
        with self.assertRaisesRegex(ValueError, 'actual source-admission rejection'):
            registered._check_actual(dict(complete=True), job)

    def test_total_missing_parent_physical_seed_does_not_cover_append(self):
        result = dict(actual={'files': {}, 'audits': []})
        with self.assertRaisesRegex(ValueError, 'every original append seed'):
            registered._append_coverage(result, 'correct')

    def test_negative_template_audits_are_registered_without_claiming_success(self):
        templates = registered.audit_templates()
        names = {Path(path).name for path in templates}
        self.assertLessEqual({'NegativeFullCapture.lean.in', 'NegativeGuard.lean.in',
                             'InboxStateWrongCurrent.lean.in', 'EnvelopeWriterWrong.lean.in'}, names)
        self.assertTrue(all(audits for audits in templates.values()))

    def test_missing_or_empty_audit_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = str(registered.HERE/'Example.lean.in')
            path = root/name
            path.parent.mkdir(parents=True)
            with patch.object(registered.subprocess, 'check_output', return_value=(name+'\0').encode()), \
                    patch.object(registered, 'AUDIT_TEMPLATE_COUNT', 1), \
                    patch.object(registered, 'AUDIT_DECLARATION_COUNT', 1):
                with self.assertRaisesRegex(ValueError, 'missing original audit template'):
                    registered.audit_template_paths(root)
                path.write_text('#audit_controller_proof Example.actual\n')
                self.assertEqual(registered.audit_template_paths(root), (name,))
                path.write_text('example : True := True.intro\n')
                with self.assertRaisesRegex(ValueError, 'missing/extra finite audit'):
                    registered.audit_template_paths(root)


if __name__ == '__main__':
    unittest.main()
