"""Finite pause-publication admission tests; never Rust or Lean evidence."""
import copy
import json
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts
from tools import proof_batch, proof_native_suite as suite
from tools.tests import test_acceptance_receipts as fixtures
from tools.tests import test_native_scopes as scope_fixtures


class PausePublicationScopeTests(unittest.TestCase):
    component = scope_fixtures.NativeScopeTests.component
    packet = scope_fixtures.NativeScopeTests.packet
    functional_fixture = scope_fixtures.NativeScopeTests.functional_fixture
    cleanup_workflow_temporary_root = scope_fixtures.NativeScopeTests.cleanup_workflow_temporary_root

    def setUp(self):
        fixtures.ReceiptTests.setUp(self)
        self.labels = receipts.PAUSE_PUBLICATION_LABELS
        temporary = mock.patch.dict(os.environ, {'TMPDIR': str(self.base)})
        temporary.start(); self.addCleanup(temporary.stop)
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c',
                        'user.email=fixture@example.invalid', 'commit', '-qm',
                        'synthetic pause-publication scope fixture'], cwd=self.source, check=True)
        rule = 'tools/tests/test_pause_publication_scope.py'
        for name, value in [('rules', {rule: receipts.sha(receipts.ROOT/rule)}),
                            ('contract_inputs', {})]:
            patch = mock.patch.object(receipts, name, return_value=value)
            patch.start(); self.addCleanup(patch.stop)

    def test_finite_packet_admits_exact_component_without_running_native_in_formal_role(self):
        seal = self.packet(scope='pause-publication')
        self.assertEqual(len(seal['commands']), 2)
        self.assertFalse(seal['full_native_suite_complete'])
        receipts.validate_native(self.cache, scope='pause-publication')
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        receipt = receipts.native_case('pause-publication', self.source, None,
                                      lambda: self.fail('formal role executed native'),
                                      scope='pause-publication')
        self.assertIn('evidence/source.tar', receipt['files'])
        self.assertIn('evidence/ignore-stop.stdout', receipt['files'])
        self.assertIn('evidence/ignore-stop.stderr', receipt['files'])
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache)

    def test_old_limited_scopes_and_missing_case_cannot_supply_publication(self):
        seal = self.packet(scope='pause-publication')
        for old in ('capture-caller', 'stage1-protocol'):
            with self.subTest(scope=old), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope=old)
            with self.subTest(scope=old), self.assertRaisesRegex(ValueError, 'outside requested'):
                receipts.native_case('pause-publication', self.source, None, scope=old)
        missing = copy.deepcopy(seal)
        for identity in list(missing['commands']):
            row = json.loads((self.cache/'commands'/identity/'receipt.json').read_text())
            if row['key']['label'] == 'pause-publication':
                missing['commands'].pop(identity)
        missing['case_names'] = list(self.labels)
        receipts.write_json(self.cache/'complete.json', missing)
        with self.assertRaisesRegex(ValueError, '2 complete components'):
            receipts.validate_native(self.cache, scope='pause-publication')

    def test_source_archive_and_both_original_streams_are_required(self):
        self.packet(scope='pause-publication')
        entry = next(path for path in (self.cache/'commands').iterdir()
                     if (path/'receipt.json').is_file() and json.loads(
                         (path/'receipt.json').read_text())['key']['label'] == 'pause-publication')
        for relative in ('evidence/source.tar', 'evidence/ignore-stop.stdout',
                         'evidence/ignore-stop.stderr'):
            path = entry/relative
            data = path.read_bytes(); path.unlink()
            with self.subTest(file=relative), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope='pause-publication')
            path.write_bytes(data)
        receipts.validate_native(self.cache, scope='pause-publication')

    def test_failed_action_cannot_publish_a_receipt_or_a_finite_seal(self):
        output = self.base/'failed'
        with self.assertRaisesRegex(RuntimeError, 'synthetic action failed'):
            self.component('pause-publication', output, scope='pause-publication', fail=True)
        key = receipts.native_key('pause-publication', ['component-case'], self.source)
        self.assertFalse((self.cache/'commands'/receipts.digest(key)/'receipt.json').exists())
        self.assertFalse((self.cache/'complete.json').exists())
        self.assertTrue((output/'fixture.log').exists())

    def test_finite_workflow_keeps_functional_and_never_runs_old_or_global_scopes(self):
        self.addCleanup(self.cleanup_workflow_temporary_root)
        def publication(work, *, scope):
            self.assertEqual(scope, 'pause-publication')
            self.component('pause-publication', work/'pause-publication', scope=scope)
        with mock.patch.object(suite, 'ROOT', self.source), \
                mock.patch.object(suite, 'rules', side_effect=receipts.rules), \
                mock.patch.object(suite, 'runtime', return_value={'unit_test_fixture': True}), \
                mock.patch.object(suite, 'functional', side_effect=self.functional_fixture), \
                mock.patch.object(suite, 'run_pause_publication_native', side_effect=publication) as current, \
                mock.patch.object(suite, 'run_capture_caller_native', side_effect=AssertionError('capture in pause scope')), \
                mock.patch.object(suite, 'run_stage1_native', side_effect=AssertionError('V3 in pause scope')), \
                mock.patch.object(suite, 'command', side_effect=AssertionError('global in pause scope')):
            suite.run_suite(self.cache, scope='pause-publication')
        current.assert_called_once()
        result = receipts.validate_native(self.cache, scope='pause-publication')
        self.assertEqual(len(result['commands']), 2)
        self.assertFalse(result['full_native_suite_complete'])

    def test_default_all_dispatches_new_component_once_and_retains_old_components(self):
        # All execution is synthetic here. Exercise the real default router,
        # preserving old routes without running Rust, Lean or component drivers.
        self.addCleanup(self.cleanup_workflow_temporary_root)
        scripts = self.source/'scripts'; scripts.mkdir()
        for name in ('controller', 'foundations', 'json-request', 'json-identifiers',
                     'network', 'first-chain-linux', 'data-drain', 'task-contract', 'json-counter'):
            (scripts/('verify-'+name+'.sh')).write_text('# synthetic router fixture\n')
        archive = self.base/'original.tar'; archive.write_bytes(b'synthetic raw input')
        environment = {name: str(archive) for name in receipts.SOURCE_ARCHIVES}
        real_run = subprocess.run
        def harmless_run(command, *args, **kwargs):
            if command[0] == 'git':
                return real_run(command, *args, **kwargs)
            return subprocess.CompletedProcess(command, 0)
        with mock.patch.dict(os.environ, environment), \
                mock.patch.object(suite, 'ROOT', self.source), \
                mock.patch.object(suite, 'trusted_checkout'), \
                mock.patch.object(suite, 'rules', side_effect=receipts.rules), \
                mock.patch.object(suite, 'runtime', return_value={'unit_test_fixture': True}), \
                mock.patch.object(suite, 'native_case') as cases, \
                mock.patch.object(suite, 'command'), \
                mock.patch.object(suite.subprocess, 'run', side_effect=harmless_run), \
                mock.patch.object(suite, 'run_capture_caller_native') as capture, \
                mock.patch.object(suite, 'run_stage1_native') as stage1, \
                mock.patch.object(suite, 'run_pause_publication_native') as publication, \
                mock.patch.object(suite, 'run_pause_stop_native') as stop, \
                mock.patch('tools.tls_pin_reproduction.reproduce'), \
                mock.patch.object(suite, 'publish_suite', return_value={'synthetic_router_only': True}):
            suite.run_suite(self.cache)
        publication.assert_called_once()
        stop.assert_called_once()
        capture.assert_called_once()
        stage1.assert_called_once()
        labels = [call.args[0] for call in cases.call_args_list]
        self.assertIn('functional', labels)
        self.assertIn('std-mutex', labels)
        self.assertIn('bounded-writer', labels)
        self.assertIn('sqlite-correct', labels)
        self.assertEqual(proof_batch.MODULES.count('pause-publication'), 1)
        self.assertEqual(proof_batch.MODULES.count('capture-caller'), 1)
        self.assertEqual(proof_batch.MODULES.count('stage1-protocol'), 1)
        self.assertFalse((self.cache/'complete.json').exists())


if __name__ == '__main__':
    unittest.main()
