import importlib.util
from copy import deepcopy
from contextlib import redirect_stdout
from io import StringIO
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
LOOKUP = Path('verification/experiments/stage1-protocol/inbox-lookup')
spec = importlib.util.spec_from_file_location(
    'inbox_lookup_module_acceptance', ROOT / LOOKUP / 'module_acceptance.py')
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


class InboxLookupModuleAcceptanceTests(unittest.TestCase):
    def test_workflow_selects_only_lookup_and_both_original_lean_views(self):
        plan = acceptance.select([str(LOOKUP / 'module_acceptance.py')])
        self.assertEqual(plan['status'], 'ready')
        self.assertEqual(plan['affected_modules'], ['inbox-lookup'])
        self.assertEqual(plan['checks'], ['original-native', 'formal-correct', 'formal-key-only'])
        self.assertIn('Inbox::find', plan['production_entry'])

    def test_isolated_mutant_template_selects_its_original_contract_check(self):
        plan = acceptance.select([str(LOOKUP / 'InboxLookupWrong.lean.in')])
        self.assertEqual(plan['status'], 'ready')
        self.assertIn('formal-key-only', plan['checks'])

    def test_wrong_template_with_an_external_lean_consumer_cannot_accept_in_isolation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / LOOKUP).mkdir(parents=True)
            acceptance.write_json(root / LOOKUP / 'inventory.json',
                                  dict(audits={'InboxLookupWrong': []}, production_inputs={}))
            outside = root / 'SandboxModel' / 'ExternalLookup.lean'
            outside.parent.mkdir()
            outside.write_text('import SandboxModel.Lifecycle.Protocol.InboxLookupWrong\n')
            self.assertIn('SandboxModel/ExternalLookup.lean',
                          acceptance.lean_consumers(root, {acceptance.WRONG_PAGE}))
            plan = acceptance.select([acceptance.WRONG_PAGE], root=root)
            self.assertEqual(plan['status'], 'review-required')
            self.assertEqual(plan['checks'], [])

    def test_shared_proof_template_selects_real_reverse_consumers_without_issuing_pass(self):
        path = str(LOOKUP / 'InboxLookupModel.lean.in')
        plan = acceptance.select([path])
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['shared_inputs'], [path])
        self.assertTrue({'create-selection', 'inbox-admission', 'inbox-record', 'inbox-position'}
                        <= set(plan['direct_consumers']))
        self.assertIn('unload-base-provenance', plan['affected_modules'])
        self.assertEqual(plan['checks'], [])

    def test_real_rust_input_and_tool_rules_must_expand(self):
        changed = [acceptance.MODEL, 'tools/acceptance_receipts.py']
        plan = acceptance.select(changed)
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['shared_inputs'], changed)
        self.assertIn('inbox-admission', plan['affected_modules'])

    def test_unknown_and_empty_changes_cannot_select_an_accidental_green(self):
        unknown = 'path/not-yet-mapped.rs'
        plan = acceptance.select([str(LOOKUP / 'module_acceptance.py'), unknown])
        self.assertEqual(plan['status'], 'review-required')
        self.assertEqual(plan['unmapped_inputs'], [unknown])
        empty = acceptance.select([])
        self.assertEqual(empty['status'], 'empty')
        self.assertEqual(empty['affected_modules'], [])
        self.assertEqual(empty['checks'], [])

    def test_positive_lean_selects_its_original_jobs_and_concrete_downstream_jobs(self):
        name = str(LOOKUP / 'InboxLookup.lean.in')
        plan = acceptance.registered_impact([name], root=ROOT)
        self.assertEqual(plan['unmapped'], [])
        self.assertEqual(plan['related_rust_checks'], [])
        jobs = {row['name']: row for row in plan['required_jobs']}
        self.assertTrue({'inbox-lookup-correct', 'inbox-lookup-key-only', 'inbox-record-correct',
                         'inbox-record-total-correct', 'create-selection', 'stage1-unload-audits'} <= jobs.keys())
        self.assertNotIn('inbox-admission-correct', jobs)
        self.assertEqual(jobs['inbox-lookup-correct']['args'], ['--require-native', '--variant', 'correct'])
        self.assertIn('stage1-inbox-append', plan['original_native_prerequisites'])
        self.assertIn('functional', plan['original_native_prerequisites'])

    def test_business_rust_plan_retains_both_crate_checks_and_source_bindings(self):
        plan = acceptance.registered_impact([acceptance.MODEL], root=ROOT)
        self.assertEqual(plan['unmapped'], [])
        self.assertIn(str(LOOKUP / 'inventory.json'), plan['registered_rust_inputs'])
        self.assertIn('controller::lifecycle_inbox::tests::', plan['related_rust_checks'][0])
        self.assertIn('clippy', plan['related_rust_checks'][1])
        self.assertIn('inbox-admission-correct', {row['name'] for row in plan['required_jobs']})
        self.assertIsNone(acceptance.registered_impact([acceptance.MODEL, 'tools/acceptance_receipts.py'], root=ROOT))

    def test_source_digest_refresh_cannot_replace_any_other_input_or_rule(self):
        before = {'variants': [{'production_inputs': {acceptance.MODEL: 'a' * 64, 'other': 'b' * 64},
                               'audit': 'kept'}]}
        actual = acceptance.refreshed_inputs(before, {acceptance.MODEL: 'c' * 64, 'audit': 'd' * 64})
        self.assertEqual(actual, {'variants': [{'production_inputs': {acceptance.MODEL: 'c' * 64, 'other': 'b' * 64},
                                                'audit': 'kept'}]})

    def test_unload_repository_manifest_refresh_keeps_the_old_module_layout(self):
        path = str(LOOKUP / 'InboxLookup.lean.in')
        before = {'modules': {'InboxLookup': {'repository': path, 'sha256': 'a' * 64},
                              'Independent': {'repository': 'other', 'sha256': 'b' * 64}}}
        self.assertEqual(acceptance.refreshed_inputs(before, {path: 'c' * 64}),
                         {'modules': {'InboxLookup': {'repository': path, 'sha256': 'c' * 64},
                                      'Independent': {'repository': 'other', 'sha256': 'b' * 64}}})

    def test_only_the_declared_lean_page_and_derived_tables_can_reuse_native(self):
        paths = sorted({acceptance.WRONG_PAGE, *acceptance.WRONG_FREEZES})
        with patch.object(acceptance, 'reviewed_refresh', return_value=True), \
                patch.object(acceptance, 'lean_consumers', return_value={'inbox-lookup'}) as consumers:
            self.assertEqual(acceptance.native_reuse_paths(paths, 'old', root=ROOT), paths)
            with self.assertRaisesRegex(ValueError, 'requires only'):
                acceptance.native_reuse_paths([*paths, acceptance.MODEL], 'old', root=ROOT)
            consumers.return_value = {'inbox-lookup', 'new-consumer'}
            with self.assertRaisesRegex(ValueError, 'unreviewed downstream'):
                acceptance.native_reuse_paths(paths, 'old', root=ROOT)

    def test_original_native_key_rejects_tool_config_argv_or_production_drift(self):
        paths = sorted({acceptance.WRONG_PAGE, *acceptance.WRONG_FREEZES})
        original = {'kind': 'inbox-lookup-native-key-v1',
                    'source': dict.fromkeys([*paths, acceptance.MODEL], 'old'),
                    'rules': {acceptance.WRONG_PAGE: 'old'},
                    'runtime': {'tools': {'cargo': 'original'}, 'test_configuration': {'threads': 4}},
                    'commands': {'test': ['cargo', 'test']}}
        current = deepcopy(original)
        current['source'].update(dict.fromkeys(paths, 'new'))
        current['rules'][acceptance.WRONG_PAGE] = 'new'
        acceptance.compare_native_keys(original, current, paths)
        variants = [
            ('runtime', {'tools': {'cargo': 'new'}, 'test_configuration': {'threads': 4}}),
            ('runtime', {'tools': {'cargo': 'original'}, 'test_configuration': {'threads': 8}}),
            ('commands', {'test': ['cargo', 'clippy']}),
            ('source', {**current['source'], acceptance.MODEL: 'drift'}),
        ]
        for key, value in variants:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                acceptance.compare_native_keys(original, {**current, key: value}, paths)

    def test_standalone_receipt_requires_its_original_result_and_every_command_log(self):
        paths = sorted({acceptance.WRONG_PAGE, *acceptance.WRONG_FREEZES})
        base = {'commit': 'a' * 40, 'tree': 'b' * 40}
        tools = {name: {'sha256': 'c' * 64, 'version': name} for name in ('cargo', 'rustc', 'clippy-driver')}
        original = {'kind': 'inbox-lookup-native-key-v1', 'source': dict.fromkeys(paths, 'old'),
                    'rules': {}, 'runtime': {'tools': {'rustup:' + name: row for name, row in tools.items()}}}
        current = {**original, 'source': dict.fromkeys(paths, 'new')}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old_module_result = root / acceptance.NATIVE_RECEIPT
            acceptance.write_json(old_module_result, dict(schema=1, kind='stage1-module-acceptance', candidate=base))
            with self.assertRaisesRegex(ValueError, 'exact original candidate receipt'):
                acceptance.load_native_reuse(old_module_result, acceptance.sha(old_module_result), base, current, paths)
            old_module_result.unlink()
            folder = root / 'original-native'
            folder.mkdir()
            (root / 'original-native.log').write_text('fixture launcher log\n')
            record = dict(kind='inbox-lookup-native-development', complete=True, inputs=original['source'],
                          source_commit=base['commit'], variants={name: {'complete': True}
                          for name in acceptance.NATIVE_VARIANTS}, registry={'test': {}}, tools=tools,
                          commands=[], seconds=1)
            for variant in acceptance.NATIVE_VARIANTS:
                (folder / variant).mkdir()
                for label in acceptance.NATIVE_COMMANDS:
                    log = folder / variant / (label + '.log')
                    log.write_text('fixture original command log\n')
                    mutant = (variant, label) == ('key-only', 'test')
                    record['commands'].append(dict(variant=variant, label=label, argv=acceptance.native_argv(label),
                        exit_code=101 if mutant else 0, property_rejected=mutant, log_sha256=acceptance.sha(log)))
            result = folder / 'result.json'
            acceptance.write_json(result, record)
            original_status = acceptance.publish_native_receipt(root, base, original, record)
            path = Path(original_status['receipt'])
            sha = original_status['receipt_sha256']
            loaded, report = acceptance.load_native_reuse(path, sha, base, current, paths)
            self.assertEqual(loaded, record)
            self.assertEqual(report['use'], 'REUSE')
            self.assertEqual(acceptance.sha(path), sha)
            self.assertFalse((root / 'complete.json').exists())
            missing = folder / 'key-only/test.log'
            missing.unlink()
            with self.assertRaisesRegex(ValueError, 'invalid original Rust command evidence'):
                acceptance.load_native_reuse(path, sha, base, current, paths)
            result.unlink()
            with self.assertRaisesRegex(ValueError, 'missing or changed original native result'):
                acceptance.load_native_reuse(path, sha, base, current, paths)

    def test_native_preparation_cli_uses_the_native_only_path_without_a_formal_lock(self):
        candidate = {'commit': 'b' * 40, 'tree': 'c' * 40}
        launcher = str(LOOKUP / 'module_acceptance.py')
        with patch.object(acceptance, 'clean_candidate', return_value=candidate), \
                patch.object(acceptance, 'changed_files', return_value=('a' * 40, [launcher])), \
                patch.object(acceptance, 'execute_native_preparation', return_value=0) as prepare, \
                patch.object(acceptance, 'execute') as formal:
            self.assertEqual(acceptance.main(['--base', 'reviewed', '--output', '/private/lookup-native',
                                              '--prepare-native']), 0)
            prepare.assert_called_once_with(Path('/private/lookup-native'), candidate)
            formal.assert_not_called()

    def test_native_preparation_preserves_status_without_any_proof_pass_report(self):
        candidate = {'commit': 'b' * 40, 'tree': 'c' * 40}
        with tempfile.TemporaryDirectory() as temp, redirect_stdout(StringIO()), \
                patch.object(acceptance, 'check_lookup_sources'), \
                patch.object(acceptance, 'prepare_native', return_value=({}, {'use': 'RUN'})) as prepare:
            output = Path(temp) / 'passed'
            self.assertEqual(acceptance.execute_native_preparation(output, candidate), 0)
            self.assertEqual(acceptance.read(output/'native-preparation.json')['status'], 'passed')
            self.assertEqual({file.name for file in output.iterdir()}, {'native-preparation.json'})
            prepare.assert_called_once_with(output.resolve(), candidate)
            prepare.side_effect = ValueError('original command failed')
            output = Path(temp) / 'failed'
            self.assertEqual(acceptance.execute_native_preparation(output, candidate), 1)
            result = acceptance.read(output/'native-preparation.json')
            self.assertEqual(result['status'], 'failed')
            self.assertEqual(result['error'], 'original command failed')
            self.assertEqual({file.name for file in output.iterdir()}, {'native-preparation.json'})

    def test_standalone_children_drop_login_session_but_keep_actual_test_configuration(self):
        candidate = {'commit': 'b' * 40, 'tree': 'c' * 40}
        launcher = str(LOOKUP / 'module_acceptance.py')
        observed = []

        def capture(*args):
            child = json.loads(subprocess.check_output([sys.executable, '-c',
                'import json, os; print(json.dumps([os.getenv("XDG_SESSION_ID"), os.getenv("RUST_TEST_THREADS")]))'],
                text=True))
            observed.append((child, dict(test_configuration=acceptance.native_test_configuration(os.environ),
                                         environment_sha256=acceptance.digest(dict(os.environ)))))
            return 0

        with tempfile.TemporaryDirectory() as temp, \
                patch.object(acceptance, 'clean_candidate', return_value=candidate), \
                patch.object(acceptance, 'changed_files', return_value=('a' * 40, [launcher])), \
                patch.object(acceptance, 'execute_native_preparation', side_effect=capture), \
                patch.object(acceptance, 'execute', side_effect=capture):
            lock = Path(temp) / 'formal.lock'
            lock.touch()
            prefix = ['--base', 'reviewed', '--output', str(Path(temp) / 'evidence')]
            for session, threads, args in [('fixture-1', '4', ['--prepare-native']),
                                           ('fixture-2', '4', ['--formal-lock', str(lock)]),
                                           ('fixture-2', '8', ['--formal-lock', str(lock)])]:
                with patch.dict(os.environ, XDG_SESSION_ID=session, RUST_TEST_THREADS=threads):
                    self.assertEqual(acceptance.main([*prefix, *args]), 0)
        self.assertEqual([child for child, _ in observed], [[None, '4'], [None, '4'], [None, '8']])
        self.assertEqual(observed[0][1], observed[1][1])
        paths = {acceptance.WRONG_PAGE, *acceptance.WRONG_FREEZES}
        original = dict(kind='inbox-lookup-native-key-v1', source=dict.fromkeys(paths, 'old'), runtime=observed[0][1])
        current = dict(original, source=dict.fromkeys(paths, 'new'), runtime=observed[2][1])
        with self.assertRaisesRegex(ValueError, 'native key changed: runtime'):
            acceptance.compare_native_keys(original, current, paths)


if __name__ == '__main__':
    unittest.main()
