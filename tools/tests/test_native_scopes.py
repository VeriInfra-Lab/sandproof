"""Synthetic admission/workflow tests; never Rust, Lean or CI evidence."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts
from tools import proof_batch, proof_native_suite as suite
from tools.tests import test_acceptance_receipts as fixtures


class NativeScopeTests(unittest.TestCase):
    def setUp(self):
        fixtures.ReceiptTests.setUp(self)
        source_admission = mock.patch.object(suite, 'stage1_source_admission')
        source_admission.start(); self.addCleanup(source_admission.stop)
        # A synthetic packet needs a synthetic fixed source identity too. The
        # actual six-member archive/physical-source gate is tested separately.
        source_only = self.base/'synthetic-create-source.tar'
        source_only.write_bytes(b'synthetic source identity; not a real source capsule\n')
        expected_source = receipts.sha(source_only)
        source_environment = mock.patch.dict(os.environ,
            SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE=str(source_only))
        source_environment.start(); self.addCleanup(source_environment.stop)
        original_registration = receipts.stage1_registration
        def registration():
            registered = original_registration()
            original_profile = registered.create_profile
            def profile(root=receipts.ROOT):
                result = original_profile(root)
                result['source_only']['archive_sha256'] = expected_source
                return result
            registered.create_profile = profile
            return registered
        registration_patch = mock.patch.object(receipts, 'stage1_registration', side_effect=registration)
        registration_patch.start(); self.addCleanup(registration_patch.stop)
        self.labels = receipts.stage1_native_labels()
        temporary = mock.patch.dict(os.environ, {'TMPDIR': str(self.base)})
        temporary.start(); self.addCleanup(temporary.stop)
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
            if label in receipts.ARCHIVED_NATIVE_CASES:
                (output/'source.tar').write_bytes(b'synthetic unit fixture archive\n')
            if label == 'capture-caller':
                (output/'ignore-capture-target.stdout').write_text('synthetic original assertion stream\n')
                (output/'ignore-capture-target.stderr').write_text('')
            if label == 'pause-publication':
                (output/'ignore-stop.stdout').write_text('synthetic original assertion stream\n')
                (output/'ignore-stop.stderr').write_text('')
            if label == 'pause-stop':
                (output/'ignore-handoff.stdout').write_text('synthetic original assertion stream\n')
                (output/'ignore-handoff.stderr').write_text('')
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
        if scope == 'stage1-create-delivery':
            seal.update(profile='create-delivery',
                create_definition=receipts.stage1_registration().create_profile())
        receipts.write_json(self.cache/'complete.json', seal)
        return seal

    def test_create_and_full_packets_cannot_substitute_for_each_other(self):
        self.labels = receipts.stage1_native_labels('create-delivery')
        self.assertEqual(len(self.labels), 13)
        self.assertNotIn('stage1-protocol', self.labels)
        seal = self.packet(scope='stage1-create-delivery')
        receipts.validate_native(self.cache, scope='stage1-create-delivery')
        for scope in ('all', 'stage1-protocol', 'capture-caller', 'pause-stop', 'pause-publication'):
            with self.subTest(scope=scope), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope=scope)
        for defect in ('missing-command', 'missing-package', 'wrong-profile', 'changed-inventory', 'missing-source'):
            bad = copy.deepcopy(seal)
            if defect == 'missing-command':
                bad['commands'].pop(next(iter(bad['commands'])))
            elif defect == 'missing-package':
                bad['create_definition']['packages'].pop()
            elif defect == 'wrong-profile':
                bad['profile'] = 'full'
            elif defect == 'missing-source':
                del bad['source_archives']['SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE']
            else:
                bad['create_definition']['packages'][0]['inventory_sha256'] = '0'*64
            receipts.write_json(self.cache/'complete.json', bad)
            with self.subTest(defect=defect), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='stage1-create-delivery')
        self.labels = receipts.stage1_native_labels()
        self.packet(scope='all')
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache, scope='stage1-create-delivery')

    def test_public_native_entry_sets_role_before_source_preparation(self):
        private = self.base/'controlled-native-temporary'
        private.mkdir()
        def admit(scope, *, prepare=False):
            self.assertTrue(prepare)
            self.assertEqual(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE'),'native')
            raise RuntimeError('controlled stop after source admission')
        for scope in ('all','stage1-protocol','stage1-create-delivery'):
            os.environ.pop('SANDPROOF_ACCEPTANCE_ROLE',None)
            with self.subTest(scope=scope), \
                    mock.patch.object(suite,'trusted_checkout'), \
                    mock.patch.object(suite,'private_temporary_root',return_value=private), \
                    mock.patch.object(suite,'stage1_source_admission',side_effect=admit), \
                    self.assertRaisesRegex(RuntimeError,'controlled stop after source admission'):
                suite.run_locked_suite(self.base/('entry-'+scope),scope=scope)

    def test_create_delegates_all_twelve_without_running_retained_or_old_drivers(self):
        registered = mock.Mock()
        work = self.base/'create-work'
        with mock.patch.object(suite, 'native_case') as retained, \
                mock.patch.object(suite, 'load', return_value=registered), \
                mock.patch.object(suite.subprocess, 'run') as run:
            suite.run_stage1_native(work, scope='stage1-create-delivery')
        retained.assert_not_called()
        run.assert_not_called()
        registered.run_native.assert_called_once_with(suite.ROOT, work/'stage1-v3',
            profile='create-delivery', scope='stage1-create-delivery')

    def test_limited_packet_admits_only_explicit_scope_and_never_executes_in_formal_role(self):
        seal = self.packet()
        self.assertEqual(len(seal['commands']), 55)
        self.assertEqual(len(set(self.labels)), 55)
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
        with self.assertRaisesRegex(ValueError, '55 complete components'):
            receipts.validate_native(self.cache, scope='stage1-protocol')

    def test_additional_component_in_limited_packet_is_rejected(self):
        self.packet(extra=True)
        with self.assertRaisesRegex(ValueError, '55 complete components'):
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
        with self.assertRaisesRegex(ValueError, '55 complete components'):
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
            self.assertEqual(root.parent, self.base)
            self.assertTrue(root.name.startswith('sandproof-native-'))
            shutil.rmtree(root)

    def test_native_parent_device_changes_invalidate_reuse_but_output_names_do_not(self):
        parent = self.base/'native-parent'; parent.mkdir(mode=0o700)
        identity = suite.temporary_parent_identity(parent)
        runtime = dict(unit_test_fixture=True, temporary_parent=identity)
        receipts.write_json(self.cache/'runtime.json', runtime)
        first = self.component('capture-caller', self.base/'first', scope='capture-caller')
        unused = self.base/'different-output-name'
        same = self.component('capture-caller', unused, scope='capture-caller')
        self.assertEqual(first['key'], same['key'])
        self.assertFalse(unused.exists())
        # A synthetic device change tests cache admission, not real FS support.
        runtime['temporary_parent'] = identity | dict(device=identity['device'] + 1)
        receipts.write_json(self.cache/'runtime.json', runtime)
        changed = self.base/'changed-device'
        later = self.component('capture-caller', changed, scope='capture-caller')
        self.assertNotEqual(first['key'], later['key'])
        self.assertTrue(changed.exists())

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
        self.assertEqual(len(seal['commands']), 55)
        self.assertEqual(len(seal['uses']), 56)
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
        registered.run_native.assert_called_once_with(suite.ROOT, work/'stage1-v3', profile='full', scope='stage1-protocol')

    def test_cli_default_stays_all_and_limited_require_is_explicit(self):
        with mock.patch.object(suite, 'run_suite') as run:
            suite.main(['run', '--output', str(self.cache)])
            run.assert_called_once_with(self.cache.resolve(), scope='all')
        with mock.patch.object(suite, 'native_case') as case:
            suite.main(['require', 'stage1-protocol', '--scope', 'stage1-protocol'])
            case.assert_called_once_with('stage1-protocol', suite.ROOT, None, scope='stage1-protocol')


class NativeTemporaryRootTests(unittest.TestCase):
    """Directory admission only; no filesystem capability or CoW claim."""

    def test_configured_parent_keeps_its_filesystem_and_creates_private_child(self):
        with tempfile.TemporaryDirectory() as name:
            parent = Path(name).resolve()
            before = parent.stat()
            with mock.patch.dict(os.environ, {'TMPDIR': str(parent)}):
                child = suite.private_temporary_root()
            try:
                self.assertEqual(child.parent, parent)
                self.assertTrue(child.name.startswith('sandproof-native-'))
                self.assertEqual(child.stat().st_mode & 0o777, 0o700)
                self.assertEqual(child.stat().st_uid, os.getuid())
                self.assertEqual(child.stat().st_dev, before.st_dev)
                self.assertEqual((parent.stat().st_dev, parent.stat().st_ino, parent.stat().st_mode),
                                 (before.st_dev, before.st_ino, before.st_mode))
            finally:
                shutil.rmtree(child)

    def test_bad_configured_parents_refuse_without_fallback_or_chmod(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            file = root/'file'; file.write_text('not a directory')
            good = root/'good'; good.mkdir(mode=0o700)
            (good/'leaf').mkdir(mode=0o700)
            link = root/'link'; link.symlink_to(good, target_is_directory=True)
            shared = root/'shared'; shared.mkdir(mode=0o770); shared.chmod(0o770)
            child = shared/'child'; child.mkdir(mode=0o700)
            choices = ['', 'relative', str(root/'missing'), str(file), str(link), str(link/'leaf'), str(shared), str(child)]
            for choice in choices:
                with self.subTest(parent=choice), mock.patch.dict(os.environ, {'TMPDIR': choice}), \
                        mock.patch.object(suite.tempfile, 'mkdtemp') as allocate:
                    with self.assertRaises((OSError, ValueError)):
                        suite.private_temporary_root()
                    allocate.assert_not_called()
            self.assertEqual(shared.stat().st_mode & 0o777, 0o770)
            self.assertTrue(link.is_symlink())


class CaptureCallerScopeTests(unittest.TestCase):
    """Fixed two-component scope only; no business or proof acceptance claim."""

    component = NativeScopeTests.component
    packet = NativeScopeTests.packet
    functional_fixture = NativeScopeTests.functional_fixture
    cleanup_workflow_temporary_root = NativeScopeTests.cleanup_workflow_temporary_root

    def setUp(self):
        fixtures.ReceiptTests.setUp(self)
        self.labels = receipts.CAPTURE_CALLER_LABELS
        temporary = mock.patch.dict(os.environ, {'TMPDIR': str(self.base)})
        temporary.start(); self.addCleanup(temporary.stop)
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c',
                        'user.email=fixture@example.invalid', 'commit', '-qm',
                        'synthetic capture scope fixture'], cwd=self.source, check=True)
        rule = 'tools/tests/test_native_scopes.py'
        for name, value in [('rules', {rule: receipts.sha(receipts.ROOT/rule)}),
                            ('contract_inputs', {})]:
            patch = mock.patch.object(receipts, name, return_value=value)
            patch.start(); self.addCleanup(patch.stop)

    def test_capture_packet_admits_only_its_complete_current_component(self):
        seal = self.packet(scope='capture-caller')
        self.assertEqual(len(seal['commands']), 2)
        self.assertFalse(seal['full_native_suite_complete'])
        receipts.validate_native(self.cache, scope='capture-caller')
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        result = receipts.native_case('capture-caller', self.source, None, scope='capture-caller')
        self.assertIn('evidence/source.tar', result['files'])
        self.assertIn('evidence/ignore-capture-target.stdout', result['files'])
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache)

    def test_wrong_scope_and_missing_case_cannot_supply_capture(self):
        seal = self.packet(scope='capture-caller')
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache, scope='stage1-protocol')
        with self.assertRaisesRegex(ValueError, 'outside requested native scope'):
            receipts.native_case('capture-caller', self.source, None, scope='stage1-protocol')
        for identity in list(seal['commands']):
            row = json.loads((self.cache/'commands'/identity/'receipt.json').read_text())
            if row['key']['label'] == 'capture-caller':
                seal['commands'].pop(identity)
        seal['case_names'] = list(self.labels)  # A claim cannot replace the receipt.
        receipts.write_json(self.cache/'complete.json', seal)
        with self.assertRaisesRegex(ValueError, '2 complete components'):
            receipts.validate_native(self.cache, scope='capture-caller')

    def test_capture_source_archive_is_mandatory_before_a_success_receipt(self):
        output = self.base/'missing-archive'
        def action():
            output.mkdir()
            (output/'fixture.log').write_text('synthetic failed admission\n')
        with self.assertRaisesRegex(ValueError, 'original regular source.tar'):
            receipts.native_case('capture-caller', self.source, output, action, scope='capture-caller')
        key = receipts.native_key('capture-caller', ['component-case'], self.source)
        self.assertFalse((self.cache/'commands'/receipts.digest(key)/'receipt.json').exists())
        self.assertTrue((output/'fixture.log').exists())

    def test_capture_target_named_evidence_is_kept_and_cargo_builds_are_not(self):
        seal = self.packet(scope='capture-caller')
        identity = next(identity for identity in seal['commands'] if json.loads(
            (self.cache/'commands'/identity/'receipt.json').read_text())['key']['label'] == 'capture-caller')
        original = self.cache/'commands'/identity/'evidence/ignore-capture-target.stdout'
        self.assertTrue(original.is_file())
        original.unlink()
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache, scope='capture-caller')
        folder = self.base/'formal-files'
        for name in ('wrong-target/actual.llbc', 'target-correct/compiler.json',
                     'formal-target/build/generated.rs', 'formal-target/.fingerprint/compiler.json'):
            path = folder/name; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('synthetic evidence selection fixture\n')
        selected = proof_batch.evidence_files(folder, include_sources=True, capture_caller=True)
        self.assertIn(str((folder/'wrong-target/actual.llbc').resolve()), selected)
        self.assertNotIn(str((folder/'target-correct/compiler.json').resolve()), selected)
        self.assertNotIn(str((folder/'formal-target/build/generated.rs').resolve()), selected)
        self.assertNotIn(str((folder/'formal-target/.fingerprint/compiler.json').resolve()), selected)

    def test_capture_workflow_never_runs_v3_or_global_commands(self):
        self.addCleanup(self.cleanup_workflow_temporary_root)
        def capture(work, *, scope):
            self.assertEqual(scope, 'capture-caller')
            self.component('capture-caller', work/'capture-caller', scope=scope)
        with mock.patch.object(suite, 'ROOT', self.source), \
                mock.patch.object(suite, 'rules', side_effect=receipts.rules), \
                mock.patch.object(suite, 'runtime', return_value={'unit_test_fixture': True}), \
                mock.patch.object(suite, 'functional', side_effect=self.functional_fixture), \
                mock.patch.object(suite, 'run_capture_caller_native', side_effect=capture) as current, \
                mock.patch.object(suite, 'run_stage1_native', side_effect=AssertionError('V3 in capture scope')), \
                mock.patch.object(suite, 'command', side_effect=AssertionError('global command in capture scope')):
            suite.run_suite(self.cache, scope='capture-caller')
        current.assert_called_once()
        result = receipts.validate_native(self.cache, scope='capture-caller')
        self.assertEqual(len(result['commands']), 2)
        self.assertFalse(result['full_native_suite_complete'])


if __name__ == '__main__':
    unittest.main()
