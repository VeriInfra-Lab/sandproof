"""Admission and scheduling regressions; no compiler, VM or proof is launched."""
import ast
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
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
            for call in calls:
                self.assertEqual({kw.arg: ast.literal_eval(kw.value) for kw in call.keywords},
                                 {'scope': 'stage1-protocol'})
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
                    registered.run_formal(Path('/unused'), Path('/unused/retained/result.json'))
            shared.assert_not_called()

    def test_formal_refuses_missing_seal_or_native_case_before_driver(self):
        for failure in ('seal', 'case'):
            def validate_native(cache, *, scope):
                self.assertEqual(scope, 'stage1-protocol')
                if failure == 'seal':
                    raise ValueError('changed seal')
            def native_case(*args, scope):
                self.assertEqual(scope, 'stage1-protocol')
                raise ValueError('missing native case')
            shared = SimpleNamespace(validate_native=validate_native, native_case=native_case)
            with patch.object(registered, '_shared', return_value=shared), patch.object(registered, '_invoke') as invoke:
                with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='formal', SANDPROOF_NATIVE_EVIDENCE='/unused'):
                    with self.assertRaisesRegex(ValueError, 'seal|native case'):
                        registered.run_formal(Path('/unused'), Path('/unused/retained/result.json'))
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

    def prepared_fixture(self, temporary):
        root = Path(temporary)/'root'
        root.mkdir()
        for name in ('verification/extract-foundations/src/lib.rs',
                     'verification/extract-foundations/Cargo.toml',
                     str(registered.HERE/'local_view.py'),
                     str(registered.HERE/'local-view.toml.in'),
                     str(registered.HERE/'schema-leaves/protocol_leaves.rs.in'),
                     str(registered.HERE/'schema-leaves/RoleLeaves.lean.in')):
            path = root/name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(registered.ROOT/name, path)
        (root/'unrelated.rs').write_text('pub const ORIGINAL: u8 = 1;\n')
        (root/'.cargo').mkdir()
        (root/'.cargo/config.toml').write_text('[build]\njobs = 4\n')
        inputs = {str(p.relative_to(root)): registered._sha(p) for p in root.rglob('*') if p.is_file()}
        output = Path(temporary)/'output'
        project = output/'preparation/source'
        shutil.copytree(root, project)
        helper = root/registered.HERE/'local_view.py'
        spec = importlib.util.spec_from_file_location('fixture_original_role_installer', helper)
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        installer.install(project, 'schema-leaves')
        return root, output, project, dict(inputs=inputs, prepared=True, complete=False)

    def test_actual_role_preparation_binds_original_complete_private_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, output, project, prepared = self.prepared_fixture(temporary)
            changes = registered._role_preparation(root, output, prepared)
            self.assertEqual(set(changes), {'lakefile.toml', 'verification/extract-foundations/src/lib.rs',
                'verification/extract-foundations/src/protocol_leaves.rs',
                'SandboxModel/Lifecycle/Protocol/RoleLeaves.lean'})
            expected = registered._check_prepared_source(root, project, prepared['inputs'], changes)
            self.assertEqual(expected['unrelated.rs'], registered._sha(root/'unrelated.rs'))

    def test_any_unpermitted_prepared_source_change_or_extra_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, output, project, prepared = self.prepared_fixture(temporary)
            changes = registered._role_preparation(root, output, prepared)
            for name in sorted(set(prepared['inputs'])-set(changes)):
                with self.subTest(source=name):
                    path = project/name
                    before = path.read_bytes()
                    path.write_bytes(before+b'\nchanged after preparation\n')
                    with self.assertRaisesRegex(ValueError, 'private prepared source differs'):
                        registered._check_prepared_source(root, project, prepared['inputs'], changes)
                    path.write_bytes(before)
            extra = project/'unregistered.rs'
            extra.write_text('pub const EXTRA: u8 = 1;\n')
            with self.assertRaisesRegex(ValueError, 'private prepared source differs'):
                registered._check_prepared_source(root, project, prepared['inputs'], changes)
            extra.unlink()
            extra.symlink_to(root/'unrelated.rs')
            with self.assertRaisesRegex(ValueError, 'symlink'):
                registered._check_prepared_source(root, project, prepared['inputs'], changes)

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

    def test_missing_changed_or_linked_parent_input_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = str(registered.HERE/'envelope-schema/inventory.json')
            metadata = root/name
            metadata.parent.mkdir(parents=True)
            parent_name = str(registered.HERE/'frozen.json')
            parent = root/parent_name
            parent.write_text('{"original":true}\n')
            original = parent.read_bytes()
            metadata.write_text(json.dumps({'inputs': {parent_name: registered._sha(parent)}}))
            with patch.object(registered, 'audit_template_paths', return_value=(str(
                    registered.HERE/'envelope-schema/EnvelopeSchema.lean.in'),)), \
                    patch.object(registered, 'formal_plan', return_value=[]), \
                    patch.object(registered.subprocess, 'check_output', return_value=(name+'\0').encode()):
                self.assertEqual(registered.validate_metadata_inputs(root), {name+':inputs': 1})
                parent.write_bytes(original+b'changed')
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)
                parent.unlink()
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)
                target = root/'same-bytes.json'
                target.write_bytes(original)
                parent.symlink_to(target)
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)

    def retained_fixture(self, temporary, *, wrong=False):
        root = Path(temporary)/'root'
        inventory = root/registered.HERE/'retained-protocol.json'
        inventory.parent.mkdir(parents=True)
        names = ['Retained'+str(i) for i in range(62)]
        inventory.write_text(json.dumps(dict(schema=1, modules=names)))
        files = {}
        for name in names:
            relative = 'SandboxModel/Lifecycle/Protocol/'+name+'.lean'
            path = root/relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('-- original '+name+'\n')
            files[relative] = registered._sha(path)
        folder = Path(temporary)/'proof-output/fixture'
        service = folder/'physical-files/service.log'
        service.parent.mkdir(parents=True)
        service.write_text('controlled validator fixture; no Lean service was invoked\n')
        result_path = folder/'result.json'
        actual = dict(files=files, audits=['Fixture.original'], service_log_sha256=registered._sha(service))
        result = dict(complete=True, actual=actual, variant='wrong' if wrong else 'correct')
        result_path.write_text(json.dumps(result))
        job = dict(name='fixture', driver='unused.py', args=['--variant', 'wrong' if wrong else 'correct'],
                   cases=['wrong' if wrong else 'correct'], actual_files=True)
        record = dict(kind='registered-stage1-formal-drivers', complete=True,
            retained_inventory_sha256=registered._sha(inventory), results=[dict(name='fixture',
                result_path=str(result_path), result_sha256=registered._sha(result_path))])
        old_path = folder.parent/'retained/formal/result.json'
        views = {}
        for profile in ('carriers', 'request', 'manifest'):
            log = old_path.parent/'correct'/profile/'physical-files/service.log'
            log.parent.mkdir(parents=True)
            log.write_text('controlled original-component provenance fixture\n')
            views[profile] = dict(files={'FixtureUnrelated.lean': '0'*64}, audits=['Fixture.retained'],
                                  service_log_sha256=registered._sha(log))
        cases = {name: dict(complete=True) for name in ('correct', 'same-incarnation', 'key-only-replay',
                  'skip-capture-binding', 'session-only-envelope')}
        cases['correct']['views'] = views
        old_path.write_text(json.dumps(dict(complete=True, all_cases_complete=True, cases=cases)))
        record['retained_result'] = dict(path=str(old_path), sha256=registered._sha(old_path))
        return root, job, record, result_path, service

    def test_retained_requires_all_62_current_exact_sources_in_correct_view(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, result_path, _ = self.retained_fixture(temporary)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                coverage = registered.compute_retained_coverage(root, record)
                self.assertEqual(len(coverage), 62)
                record['retained_source_files'] = coverage
                self.assertEqual(registered.validate_retained_coverage(root, record), coverage)
                result = json.loads(result_path.read_text())
                missing = next(iter(result['actual']['files']))
                result['actual']['files'].pop(missing)
                result_path.write_text(json.dumps(result))
                record['results'][0]['result_sha256'] = registered._sha(result_path)
                with self.assertRaisesRegex(ValueError, 'lacks exact correct-view coverage'):
                    registered.compute_retained_coverage(root, record)

    def retained_audit_fixture(self, temporary):
        _, _, record, _, _ = self.retained_fixture(temporary)
        path = Path(record['retained_result']['path'])
        result = json.loads(path.read_text())
        expected = set()
        for case, row in result['cases'].items():
            field = 'views' if case == 'correct' else 'proofs'
            if field == 'proofs':
                row[field] = {'Negative': {}}
            for view in row[field]:
                audit = 'Fixture.' + case.replace('-', '_') + '.' + view
                expected.add(audit)
                log = path.parent/case/view/'physical-files/service.log'
                log.parent.mkdir(parents=True, exist_ok=True)
                log.write_text('Controlled fixture; no Lean service invoked.\n'
                    + 'Checked ' + audit + ': 3 standard logical dependencies\n')
                row[field][view] = dict(files={'Fixture.lean': '0'*64}, audits=[audit],
                                       service_log_sha256=registered._sha(log))
        path.write_text(json.dumps(result))
        record['retained_result']['sha256'] = registered._sha(path)
        return record, path, result, expected

    def test_retained_positive_and_negative_service_audits_are_both_collected(self):
        with tempfile.TemporaryDirectory() as temporary:
            record, _, _, expected = self.retained_audit_fixture(temporary)
            self.assertEqual(registered.retained_audits(record), expected)
            self.assertIn('Fixture.same_incarnation.Negative', expected)
            self.assertIn('Fixture.key_only_replay.Negative', expected)

    def test_retained_audit_record_does_not_replace_missing_or_changed_service(self):
        for change in ('missing', 'changed', 'linked', 'missing-message', 'missing-files', 'missing-views'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                record, path, result, _ = self.retained_audit_fixture(temporary)
                row = result['cases']['key-only-replay']
                log = path.parent/'key-only-replay/Negative/physical-files/service.log'
                if change == 'missing':
                    log.unlink()
                elif change == 'changed':
                    log.write_text('changed service')
                elif change == 'linked':
                    target = log.with_suffix('.original')
                    log.rename(target)
                    log.symlink_to(target)
                elif change == 'missing-message':
                    log.write_text('No target audit was actually reported.\n')
                    row['proofs']['Negative']['service_log_sha256'] = registered._sha(log)
                elif change == 'missing-files':
                    row['proofs']['Negative']['files'] = {}
                else:
                    row['proofs'] = {}
                path.write_text(json.dumps(result))
                record['retained_result']['sha256'] = registered._sha(path)
                with self.assertRaisesRegex(ValueError, 'retained'):
                    registered.retained_audits(record)

    def test_retained_wrong_view_cannot_back_current_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary, wrong=True)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                with self.assertRaisesRegex(ValueError, 'lacks exact correct-view coverage'):
                    registered.compute_retained_coverage(root, record)

    def test_extra_unregistered_protocol_source_is_rejected_even_on_reuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                record['retained_source_files'] = registered.compute_retained_coverage(root, record)
                (root/'SandboxModel/Lifecycle/Protocol/Unregistered.lean').write_text('-- extra source\n')
                with self.assertRaisesRegex(ValueError, 'unregistered or missing retained protocol'):
                    registered.validate_retained_coverage(root, record)

    def test_original_carriers_view_can_supply_its_own_exact_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, result_path, _ = self.retained_fixture(temporary)
            current = json.loads(result_path.read_text())
            name = next(iter(current['actual']['files']))
            digest = current['actual']['files'].pop(name)
            result_path.write_text(json.dumps(current))
            record['results'][0]['result_sha256'] = registered._sha(result_path)
            old_path = Path(record['retained_result']['path'])
            old = json.loads(old_path.read_text())
            old['cases']['correct']['views']['carriers']['files'][name] = digest
            old_path.write_text(json.dumps(old))
            record['retained_result']['sha256'] = registered._sha(old_path)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                coverage = registered.compute_retained_coverage(root, record)
                self.assertEqual(coverage[name]['job'], 'retained-stage1-protocol')
                self.assertEqual(coverage[name]['view'], 'carriers')

    def test_incomplete_original_component_or_missing_correct_view_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary)
            path = Path(record['retained_result']['path'])
            original = json.loads(path.read_text())
            for field in ('case', 'view', 'complete'):
                changed = copy.deepcopy(original)
                if field == 'case':
                    changed['cases'].pop('same-incarnation')
                elif field == 'view':
                    changed['cases']['correct']['views'].pop('carriers')
                else:
                    changed['all_cases_complete'] = False
                path.write_text(json.dumps(changed))
                record['retained_result']['sha256'] = registered._sha(path)
                with patch.object(registered, 'formal_plan', return_value=[job]):
                    with self.assertRaisesRegex(ValueError, 'all original cases and correct views'):
                        registered.compute_retained_coverage(root, record)

    def test_retained_changed_result_service_source_or_coverage_is_rejected(self):
        for change in ('result', 'service', 'source', 'coverage'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root, job, record, result_path, service = self.retained_fixture(temporary)
                with patch.object(registered, 'formal_plan', return_value=[job]):
                    record['retained_source_files'] = registered.compute_retained_coverage(root, record)
                    if change == 'result':
                        result_path.write_text(result_path.read_text()+'\n')
                    elif change == 'service':
                        service.write_text('changed original service log')
                    elif change == 'source':
                        (root/next(iter(record['retained_source_files']))).write_text('changed source')
                    else:
                        record['retained_source_files'].pop(next(iter(record['retained_source_files'])))
                    with self.assertRaises(ValueError):
                        registered.validate_retained_coverage(root, record)


if __name__ == '__main__':
    unittest.main()
