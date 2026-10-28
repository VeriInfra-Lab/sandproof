"""Synthetic admission/workflow tests; never Rust, Lean or CI evidence."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts
from tools import proof_batch, proof_native_suite as suite
from tools.tests import test_acceptance_receipts as fixtures


class NativeScopeTests(unittest.TestCase):
    def setUp(self):
        fixtures.ReceiptTests.setUp(self)
        self.labels = receipts.stage1_native_labels()
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c',
                        'user.email=fixture@example.invalid', 'commit', '-qm',
                        'synthetic scope fixture'], cwd=self.source, check=True)
        # These tests isolate packet scope/integrity, not real runtime or
        # source-rule completeness. The general receipt tests cover those.
        rule = 'tools/tests/test_native_scopes.py'
        for name, value in [('rules', {rule: receipts.sha(receipts.ROOT/rule)}),
                            ('contract_inputs', {})]:
            patch = mock.patch.object(receipts, name, return_value=value)
            patch.start(); self.addCleanup(patch.stop)

    def component(self, label, output, *, scope='all', fail=False):
        def action():
            output.mkdir()
            (output/'fixture.log').write_text('synthetic admission action\n')
            if fail:
                raise RuntimeError('synthetic action failed')
            if label == 'stage1-protocol':
                (output/'source.tar').write_bytes(b'synthetic unit fixture archive\n')
        return receipts.native_case(label, self.source, output, action, scope=scope)

    def packet(self, *, scope='stage1-protocol', extra=False, atomic=False):
        commands = {}
        for label in (*self.labels, *(('foreign-component',) if extra else ())):
            receipt = self.component(label, self.base/label)
            entry = self.cache/'commands'/receipts.digest(receipt['key'])
            commands[entry.name] = receipts.sha(entry/'receipt.json')
        if atomic:
            receipts.native_run('fixture', self.command, self.source, self.log)
            key = receipts.native_key('fixture', self.command, self.source)
            entry = self.cache/'commands'/receipts.digest(key)
            commands[entry.name] = receipts.sha(entry/'receipt.json')
        seal = dict(schema=receipts.NATIVE_SCHEMA, scope=scope, scope_complete=True,
            full_native_suite_complete=scope=='all', complete=True, exit_code=0,
            rules=receipts.rules(), rust=receipts.rust_inputs(self.source),
            contracts=receipts.contract_inputs(receipts.ROOT),
            runtime=json.loads((self.cache/'runtime.json').read_text()),
            source_archives=receipts.source_archive_inputs(), commands=commands)
        receipts.write_json(self.cache/'complete.json', seal)
        return seal

    def test_limited_packet_admits_only_explicit_scope_and_never_executes_in_formal_role(self):
        seal = self.packet()
        self.assertEqual(len(seal['commands']), 43)
        self.assertEqual(len(set(self.labels)), 43)
        self.assertLessEqual({'stage1-'+name for name in (
            'inbox-entry-validation', 'inbox-retired-identities', 'inbox-origin-selection',
            'inbox-config', 'inbox-config-validation', 'terminal-binding', 'retention-lane',
            'native-retirement', 'session-shape', 'inbox-validation', 'inbox-save',
            'inbox-record-validation', 'inbox-load')}, set(self.labels))
        self.assertTrue(receipts.validate_native(self.cache, scope='stage1-protocol')['scope_complete'])
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        with mock.patch.object(receipts.subprocess, 'run', wraps=subprocess.run) as run:
            receipt = receipts.native_case(self.labels[2], self.source, None, scope='stage1-protocol')
        self.assertEqual(receipt['key']['label'], self.labels[2])
        self.assertTrue(all(call.args[0][0] == 'git' for call in run.call_args_list))
        for admit in (lambda: receipts.validate_native(self.cache),
                      lambda: receipts.native_case(self.labels[2], self.source, None)):
            with self.assertRaises(ValueError):
                admit()

    def test_atomic_commands_are_allowed_but_default_native_run_still_requires_all(self):
        self.packet(atomic=True)
        receipts.validate_native(self.cache, scope='stage1-protocol')
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        with self.assertRaises(ValueError):
            receipts.native_run('fixture', self.command, self.source, self.log)

    def test_all_packet_can_supply_stage1_but_scoped_request_refuses_foreign_label(self):
        self.packet(scope='all', extra=True)
        receipts.validate_native(self.cache)
        receipts.validate_native(self.cache, scope='stage1-protocol')
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        receipts.native_case(self.labels[2], self.source, None, scope='stage1-protocol')
        with self.assertRaisesRegex(ValueError, 'outside requested native scope'):
            receipts.native_case('foreign-component', self.source, None, scope='stage1-protocol')

    def test_missing_component_cannot_be_replaced_by_a_claimed_label_list(self):
        seal = self.packet()
        missing = copy.deepcopy(seal)
        missing['case_names'] = list(self.labels)
        missing['commands'].pop(next(iter(missing['commands'])))
        receipts.write_json(self.cache/'complete.json', missing)
        with self.assertRaisesRegex(ValueError, '43 complete components'):
            receipts.validate_native(self.cache, scope='stage1-protocol')

    def test_additional_component_in_limited_packet_is_rejected(self):
        self.packet(extra=True)
        with self.assertRaisesRegex(ValueError, '43 complete components'):
            receipts.validate_native(self.cache, scope='stage1-protocol')

    def test_two_different_receipts_for_one_component_are_not_two_required_components(self):
        seal = self.packet()
        source = self.source/'lib.rs'; original = source.read_bytes()
        source.write_bytes(original+b'// another fixture input\n')
        receipt = self.component('functional', self.base/'duplicate-functional')
        source.write_bytes(original)
        entry = self.cache/'commands'/receipts.digest(receipt['key'])
        seal['commands'][entry.name] = receipts.sha(entry/'receipt.json')
        receipts.write_json(self.cache/'complete.json', seal)
        with self.assertRaisesRegex(ValueError, '43 complete components'):
            receipts.validate_native(self.cache, scope='stage1-protocol')

    def test_corrupt_or_missing_raw_evidence_prevents_any_scoped_admission(self):
        seal = self.packet()
        identity = next(iter(seal['commands']))
        entry = self.cache/'commands'/identity
        raw = next(path for path in (entry/'evidence').rglob('*') if path.is_file())
        original = raw.read_bytes()
        for damaged in (b'corrupt raw fixture', None):
            with self.subTest(damaged=damaged):
                if damaged is None:
                    raw.unlink()
                else:
                    raw.write_bytes(damaged)
                with self.assertRaises(ValueError):
                    receipts.validate_native(self.cache, scope='stage1-protocol')
                os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
                with self.assertRaises(ValueError):
                    receipts.native_case(self.labels[2], self.source, None, scope='stage1-protocol')
                raw.write_bytes(original)

    def test_wrong_or_missing_scope_and_contradictory_completion_flags_are_rejected(self):
        seal = self.packet()
        for field, value in [('scope', None), ('scope', 'unknown'),
                             ('scope_complete', False), ('full_native_suite_complete', True)]:
            bad = copy.deepcopy(seal); bad[field] = value
            receipts.write_json(self.cache/'complete.json', bad)
            with self.subTest(field=field), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='stage1-protocol')

    def test_global_formal_batch_rejects_limited_packet_before_starting_tools(self):
        self.packet()
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        with mock.patch.object(proof_batch, 'runtime') as runtime, \
                mock.patch.object(proof_batch, 'step') as step:
            with self.assertRaises(ValueError):
                proof_batch.run_batch(self.base/'formal-cache')
        runtime.assert_not_called(); step.assert_not_called()
        self.assertFalse((self.base/'formal-cache').exists())

    def test_formal_dependency_identity_includes_scope(self):
        seal = self.packet()
        scoped = proof_batch.native_dependencies(self.cache)
        seal.update(scope='all', full_native_suite_complete=True)
        receipts.write_json(self.cache/'complete.json', seal)
        full = proof_batch.native_dependencies(self.cache)
        self.assertNotEqual(receipts.digest(scoped), receipts.digest(full))
        self.assertEqual(scoped['scope'], 'stage1-protocol')
        self.assertEqual(full['scope'], 'all')

    def test_foreign_scope_action_is_refused_before_execution(self):
        output = self.base/'foreign'
        with self.assertRaisesRegex(ValueError, 'outside requested native scope'):
            self.component('foreign-component', output, scope='stage1-protocol')
        self.assertFalse(output.exists())

    def functional_fixture(self, work):
        target = work/'functional'; target.mkdir()
        (target/'check-function.log').write_text('synthetic workflow fixture, not functional acceptance\n')

    def cleanup_workflow_temporary_root(self):
        record = self.cache/'temporary-root.json'
        if record.is_file():
            root = Path(json.loads(record.read_text())['path'])
            self.assertEqual(root.parent, Path('/tmp'))
            self.assertTrue(root.name.startswith('sandproof-native-'))
            shutil.rmtree(root)

    def test_limited_workflow_seals_only_real_success_receipts_and_allows_duplicate_uses(self):
        self.addCleanup(self.cleanup_workflow_temporary_root)
        def actions(work, *, scope):
            self.assertEqual(scope, 'stage1-protocol')
            for label in self.labels[1:]:
                self.component(label, work/label, scope=scope)
            receipts.native_case('stage1-protocol', self.source, work/'unused',
                lambda: self.fail('complete retained action executed twice'), scope=scope)
        with mock.patch.object(suite, 'ROOT', self.source), \
                mock.patch.object(suite, 'rules', side_effect=receipts.rules), \
                mock.patch.object(suite, 'runtime', return_value={'unit_test_fixture': True}), \
                mock.patch.object(suite, 'functional', side_effect=self.functional_fixture), \
                mock.patch.object(suite, 'run_stage1_native', side_effect=actions), \
                mock.patch.object(suite, 'command', side_effect=AssertionError('global command in limited run')):
            suite.run_suite(self.cache, scope='stage1-protocol')
        seal = receipts.validate_native(self.cache, scope='stage1-protocol')
        self.assertEqual(len(seal['commands']), 43)
        self.assertEqual(len(seal['uses']), 44)
        self.assertEqual(sum(row['reused'] for row in seal['uses']), 1)
        self.assertTrue(seal['scope_complete'])
        self.assertFalse(seal['full_native_suite_complete'])

    def test_failed_action_never_publishes_limited_seal(self):
        self.addCleanup(self.cleanup_workflow_temporary_root)
        with mock.patch.object(suite, 'ROOT', self.source), \
                mock.patch.object(suite, 'rules', side_effect=receipts.rules), \
                mock.patch.object(suite, 'runtime', return_value={'unit_test_fixture': True}), \
                mock.patch.object(suite, 'functional', side_effect=self.functional_fixture), \
                mock.patch.object(suite, 'run_stage1_native', side_effect=RuntimeError('action failed')):
            with self.assertRaisesRegex(RuntimeError, 'action failed'):
                suite.run_suite(self.cache, scope='stage1-protocol')
        self.assertFalse((self.cache/'complete.json').exists())

    def test_scoped_failed_action_keeps_raw_log_without_a_success_receipt(self):
        label = self.labels[2]; output = self.base/'failed-action'
        with self.assertRaisesRegex(RuntimeError, 'synthetic action failed'):
            self.component(label, output, scope='stage1-protocol', fail=True)
        key = receipts.native_key(label, ['component-case'], self.source)
        self.assertFalse((self.cache/'commands'/receipts.digest(key)/'receipt.json').exists())
        self.assertTrue((output/'fixture.log').is_file())
        self.assertFalse((self.cache/'complete.json').exists())

    def test_original_retained_action_and_all_registered_drivers_are_delegated_unchanged(self):
        work = self.base/'work'
        registered = mock.Mock()
        with mock.patch.object(suite, 'native_case') as case, \
                mock.patch.object(suite, 'load', return_value=registered), \
                mock.patch.object(suite.subprocess, 'run') as run:
            suite.run_stage1_native(work, scope='stage1-protocol')
            args = case.call_args
            self.assertEqual(args.args[:3], ('stage1-protocol', suite.ROOT, work/'stage1-protocol'))
            self.assertEqual(args.kwargs, {'scope': 'stage1-protocol'})
            args.args[3]()
        self.assertEqual(run.call_args.args[0][1:], [
            'verification/experiments/stage1-protocol/component.py',
            '--native-only', '--output', str(work/'stage1-protocol')])
        registered.run_native.assert_called_once_with(suite.ROOT, work/'stage1-v3')

    def test_cli_default_stays_all_and_limited_require_is_explicit(self):
        with mock.patch.object(suite, 'run_suite') as run:
            suite.main(['run', '--output', str(self.cache)])
            run.assert_called_once_with(self.cache.resolve(), scope='all')
        with mock.patch.object(suite, 'native_case') as case:
            suite.main(['require', 'stage1-protocol', '--scope', 'stage1-protocol'])
            case.assert_called_once_with('stage1-protocol', suite.ROOT, None, scope='stage1-protocol')


if __name__ == '__main__':
    unittest.main()
