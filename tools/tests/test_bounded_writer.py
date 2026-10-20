"""Admission regressions: preserve source/IR/specification, not an alternate Writer."""
import copy
import json
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

from tools import check_bounded_writer as checker

ROOT = Path(__file__).resolve().parents[2]


class BoundedWriterAdmission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.correct = checker.read(ROOT / checker.ASSETS / 'positive/original.llbc')
        cls.wrong = checker.read(ROOT / checker.ASSETS / 'negative/original.llbc')

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='bounded-writer-admission-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        path = self.root / checker.ENVIRONMENT
        path.parent.mkdir(parents=True)
        shutil.copyfile(ROOT / checker.ENVIRONMENT, path)
        self.llbc = copy.deepcopy(self.correct)

    def reject(self, data=None, variant='correct'):
        with self.assertRaises(ValueError):
            checker.check_ir(self.root, self.llbc if data is None else data, variant)

    def test_complete_original_ir_admitted(self):
        checker.check_ir(self.root, self.correct, 'correct')
        checker.check_ir(self.root, self.wrong, 'no-limit')

    def test_only_declared_unordered_short_name_order_may_change(self):
        self.llbc['translated']['short_names'] = [
            {'key': {'Fun': 11}, 'value': 'write_entry'},
            {'key': {'Fun': 12}, 'value': 'finish_entry'},
        ]
        before = checker.llbc_digest(self.llbc)
        self.llbc['translated']['short_names'].reverse()
        self.assertEqual(before, checker.llbc_digest(self.llbc))

    def test_duplicate_short_name_is_rejected(self):
        sample = {'key': {'Fun': 11}, 'value': 'write_entry'}
        self.llbc['translated']['short_names'] = [sample, copy.deepcopy(sample)]
        with self.assertRaisesRegex(ValueError, 'duplicate short-name key'):
            checker.check_ir(self.root, self.llbc, 'correct')

    def test_ordered_function_array_cannot_be_reordered(self):
        self.llbc['translated']['fun_decls'].reverse()
        self.reject()

    def test_owned_limit_policy_cannot_be_opaque(self):
        functions = self.llbc['translated']['fun_decls']
        found = [f for f in functions if f and {'Ident': ['write_bounded', 0]} in f['item_meta']['name']
                 and isinstance(f.get('body'), dict)]
        self.assertTrue(found)
        found[0]['body'] = 'Opaque'
        self.reject()

    def test_drop_paths_cannot_be_removed(self):
        functions = self.llbc['translated']['fun_decls']
        found = [f for f in functions if f and 'DropGlue' in json.dumps(f['item_meta']['name'])]
        self.assertTrue(found)
        functions.remove(found[0])
        self.reject()

    def test_source_location_is_not_normalized_away(self):
        files = self.llbc['translated']['files']
        found = [f for f in files if f and isinstance(f.get('name'), dict) and 'Local' in f['name']]
        self.assertTrue(found)
        found[0]['name']['Local'] += '.unregistered-location'
        self.reject()

    def test_borrow_check_cannot_be_disabled(self):
        self.llbc['translated']['options']['skip_borrowck'] = True
        self.reject()

    def test_owned_helper_cannot_be_added_to_opaque_boundary(self):
        self.llbc['translated']['options']['opaque'].append('sandproof_controller_service::template_registry::evidence::write_bounded')
        self.reject()

    def test_wrong_rust_cannot_be_admitted_as_correct(self):
        self.reject(self.wrong)
        self.reject(self.correct, 'no-limit')

    def generated_fixture(self):
        # A small unit fixture; actual replay checks the full frozen emitted set,
        # including the unused external-type template's registered content hash.
        generated = self.root / 'fresh-generated'
        files = ['SandproofBoundedWriter/Types.lean', 'SandproofBoundedWriter/Funs.lean', 'translation.json']
        for name in files:
            a = self.root / checker.LIBRARY / name
            b = generated / name
            a.parent.mkdir(parents=True, exist_ok=True)
            b.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / checker.LIBRARY / name, a)
            shutil.copyfile(a, b)
        profile = checker.read(self.root / checker.ENVIRONMENT)
        profile['variants']['correct']['generated'] = {name: checker.sha(generated / name) for name in files}
        (self.root / checker.ENVIRONMENT).write_text(json.dumps(profile))
        checker.check_generated(self.root, generated, 'correct')
        return generated

    def test_extra_generated_file_is_rejected(self):
        generated = self.generated_fixture()
        (generated / 'unregistered.lean').write_text('import Aeneas\n')
        with self.assertRaises(ValueError):
            checker.check_generated(self.root, generated, 'correct')

    def test_changed_generated_policy_is_rejected(self):
        generated = self.generated_fixture()
        path = generated / 'SandproofBoundedWriter/Funs.lean'
        path.write_text(path.read_text().replace('exceeded := true', 'exceeded := false', 1))
        with self.assertRaises(ValueError):
            checker.check_generated(self.root, generated, 'correct')

    def test_same_property_cannot_be_weakened_for_negative(self):
        for relative in [checker.MODELS, checker.ASSETS / 'positive', checker.ASSETS / 'negative']:
            shutil.copytree(ROOT / relative, self.root / relative)
        external = self.root / checker.LIBRARY / 'SandproofBoundedWriter/TypesExternal.lean'
        external.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / checker.LIBRARY / 'SandproofBoundedWriter/TypesExternal.lean', external)
        checker.check_pair(self.root)
        model = self.root / checker.MODELS / 'Model.lean'
        before = model.read_text()
        self.assertIn('after.limit = before.limit', before)
        model.write_text(before.replace('after.limit = before.limit', 'True', 1))
        with self.assertRaises(ValueError):
            checker.check_pair(self.root)

    def test_independent_contract_does_not_import_target_algorithm(self):
        for name in ('Model.lean', 'Contract.lean', 'Library.lean'):
            imports = [line for line in (ROOT / checker.MODELS / name).read_text().splitlines()
                       if line.startswith('import ')]
            self.assertNotIn('import SandproofBoundedWriter.Funs', imports)


class NativeArtifactIdentity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('bounded_writer_artifacts',
            ROOT / checker.ASSETS / 'native_artifacts.py')
        cls.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.helper)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='bounded-writer-cargo-identity-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.target = self.root / 'target'
        self.target.mkdir()
        self.identities = {}
        self.rows = []
        for name, (manifest, source) in self.helper.OWN.items():
            for path in (self.root / manifest, self.root / source):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('fixture source')
            package_id = 'current-source:' + name
            data = dict(name=name, manifest_path=str(self.root / manifest), src_path=str(self.root / source),
                        target_name=name.replace('-', '_'))
            self.identities[package_id] = data
            artifact = self.target / ('lib' + data['target_name'] + '.rlib')
            artifact.write_bytes(name.encode())
            self.rows.append(dict(reason='compiler-artifact', package_id=package_id,
                manifest_path=data['manifest_path'], target=dict(name=data['target_name'],
                src_path=data['src_path'], kind=['lib']), fresh=False, filenames=[str(artifact)], executable=None))
        source = self.root / self.helper.TESTS['original_writer']
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text('fixture test source')
        binary = self.target / 'original_writer'
        binary.write_bytes(b'fixture executable, not a Rust proof')
        wrapper = 'current-source:sandproof-bounded-writer-extract'
        self.rows.append(dict(reason='compiler-artifact', package_id=wrapper,
            manifest_path=self.identities[wrapper]['manifest_path'], target=dict(name='original_writer',
            src_path=str(source), kind=['test']), fresh=False, filenames=[str(binary)], executable=str(binary)))

    def check(self):
        log = self.root / 'cargo.jsonl'
        log.write_text(''.join(json.dumps(row)+'\n' for row in self.rows))
        return self.helper.build_artifacts(self.root, self.target, log, self.identities, ['original_writer'])

    def test_fresh_library_and_test_identity_admitted(self):
        result = self.check()
        self.assertEqual(set(result['libraries']), set(self.helper.OWN))
        self.assertEqual(set(result['tests']), {'original_writer'})

    def test_new_test_does_not_make_cached_library_acceptable(self):
        self.rows[0]['fresh'] = True
        with self.assertRaisesRegex(ValueError, 'cached'):
            self.check()

    def test_same_name_different_package_source_id_rejected(self):
        self.rows[0]['package_id'] = 'another-source:sandproof-controller-service'
        with self.assertRaisesRegex(ValueError, 'another package/source ID'):
            self.check()

    def test_library_manifest_from_previous_variant_rejected(self):
        self.rows[0]['manifest_path'] = str(self.root / 'old-variant/Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'manifest'):
            self.check()

    def test_library_src_from_previous_variant_rejected(self):
        old = self.root / 'previous-lib.rs'
        old.write_text('other variant')
        self.rows[0]['target']['src_path'] = str(old)
        with self.assertRaisesRegex(ValueError, 'src_path'):
            self.check()

    def test_test_src_from_previous_variant_rejected(self):
        old = self.root / 'previous-test.rs'
        old.write_text('other variant')
        self.rows[-1]['target']['src_path'] = str(old)
        with self.assertRaisesRegex(ValueError, 'src_path'):
            self.check()

    def test_cache_isolation_leaves_dependencies_and_similar_package_names(self):
        profile = self.target / 'debug'
        own = profile / '.fingerprint/sandproof-internal-0123456789abcdef'
        other = profile / '.fingerprint/sandproof-internal-helper-0123456789abcdef'
        for path in (own, other):
            path.mkdir(parents=True)
            (path / 'metadata').write_text(path.name)
        dependency = profile / 'deps/libserde-0123456789abcdef.rlib'
        dependency.parent.mkdir()
        dependency.write_bytes(b'keep dependency')
        owned = profile / 'deps/libsandproof_internal-0123456789abcdef.rlib'
        owned.write_bytes(b'isolate owned variant')
        moved = self.helper.quarantine_owned_cache(self.target, self.root / 'quarantine')
        self.assertIn('debug/.fingerprint/sandproof-internal-0123456789abcdef', moved)
        self.assertFalse(own.exists())
        self.assertFalse(owned.exists())
        self.assertTrue(other.exists())
        self.assertEqual(dependency.read_bytes(), b'keep dependency')


class RuntimeDirectoryLease(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('bounded_writer_lease',
            ROOT / checker.ASSETS / 'lease.py')
        cls.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.helper)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='bounded-writer-lease-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.paths = {}
        rows = {}
        for role in self.helper.ROLES:
            path = self.root / role
            path.mkdir()
            identity = path.stat()
            self.paths[role] = path
            rows[role] = dict(physical_path='/data1/zn/allocated-' + role,
                runtime_path=str(path), st_dev=identity.st_dev, st_ino=identity.st_ino)
        self.document = dict(schema=self.helper.SCHEMA, directories=rows,
                             bwrap_sha256=self.helper.BWRAP_SHA256)

    def test_exact_three_host_directory_identities_admitted(self):
        self.assertEqual(self.helper.validate(self.document), self.paths)

    def test_same_text_new_directory_inode_rejected(self):
        self.paths['work'].rename(self.root / 'previous-work')
        self.paths['work'].mkdir()
        with self.assertRaisesRegex(ValueError, 'differs from host directory'):
            self.helper.validate(self.document)

    def test_runtime_symlink_alias_rejected(self):
        alias = self.root / 'alias'
        alias.symlink_to(self.paths['source'])
        self.document['directories']['source']['runtime_path'] = str(alias)
        with self.assertRaisesRegex(ValueError, 'exact real directory'):
            self.helper.validate(self.document)

    def test_wrong_device_rejected(self):
        self.document['directories']['target']['st_dev'] += 1
        with self.assertRaisesRegex(ValueError, 'differs from host directory'):
            self.helper.validate(self.document)

    def test_disk_root_or_outside_data1_not_an_admissible_physical_mapping(self):
        for name in ('/data1/zn', '/tmp/allocated-source'):
            self.document['directories']['source']['physical_path'] = name
            with self.assertRaises(ValueError):
                self.helper.validate(self.document)

    def test_source_work_target_cannot_overlap(self):
        self.document['directories']['work']['physical_path'] = '/data1/zn/allocated-source/sub'
        with self.assertRaisesRegex(ValueError, 'non-nested'):
            self.helper.validate(self.document)

    def test_unregistered_tool_or_fourth_directory_rejected(self):
        for key, value in [('bwrap_sha256', '0' * 64), ('schema', 'unregistered')]:
            document = copy.deepcopy(self.document)
            document[key] = value
            with self.assertRaisesRegex(ValueError, 'schema/tool'):
                self.helper.validate(document)
        self.document['directories']['seed'] = dict(self.document['directories']['source'])
        with self.assertRaisesRegex(ValueError, 'schema/tool'):
            self.helper.validate(self.document)

    def test_chmod_is_not_a_read_only_mount(self):
        lease = self.root / 'lease.json'
        lease.write_text(json.dumps(self.document))
        lease.chmod(0o444)
        with self.assertRaisesRegex(ValueError, 'mounted read-only'):
            self.helper.admit(lease, Path('/bin/true'))


class ActualServiceAuditMessages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('bounded_writer_audit_messages',
            ROOT / checker.ASSETS / 'audit_messages.py')
        cls.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.helper)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='bounded-writer-audit-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / 'Proof.lean'
        self.source.write_text('#audit_controller_proof Example.bound\n')
        self.log = self.root / 'file-0.log'
        self.message = 'Checked Example.bound: 2 standard logical dependencies'
        self.evidence = dict(file='Proof.lean', source_sha256=checker.sha(self.source),
                             complete=True, audits=1)
        self.row = dict(request={'tool': 'lean_diagnostic_messages',
            'arguments': {'file_path': 'Proof.lean'}, 'expected_sha256': self.evidence['source_sha256']},
            stale=False, revision={'file_sha256': self.evidence['source_sha256']},
            result={'success': True, 'partial': False, 'timed_out': False,
                    'items': [{'severity': 'information', 'message': self.message}]})
        self.save_log()

    def save_log(self):
        # Preserve a representative unrelated client-cleanup stderr line as well.
        self.log.write_text(json.dumps(self.row) + '\nRuntimeError: Event loop is closed\n')
        self.evidence['log_sha256'] = checker.sha(self.log)

    def test_forwards_original_message_with_actual_dependency_count(self):
        self.assertEqual(self.helper.read_messages(self.source, self.log, self.evidence), [self.message])

    def test_declared_name_without_real_message_is_rejected(self):
        self.row['result']['items'] = []
        self.save_log()
        with self.assertRaisesRegex(ValueError, 'missing, duplicated or extra'):
            self.helper.read_messages(self.source, self.log, self.evidence)

    def test_extra_or_duplicate_actual_message_is_rejected(self):
        for name in ('Example.extra', 'Example.bound'):
            self.row['result']['items'] = [{'message': self.message},
                {'message': 'Checked ' + name + ': 1 standard logical dependencies'}]
            self.save_log()
            with self.assertRaisesRegex(ValueError, 'missing, duplicated or extra'):
                self.helper.read_messages(self.source, self.log, self.evidence)

    def test_changed_service_log_is_rejected(self):
        self.log.write_text(self.log.read_text() + 'changed\n')
        with self.assertRaisesRegex(ValueError, 'evidence changed'):
            self.helper.read_messages(self.source, self.log, self.evidence)

    def test_partial_stale_or_warning_cannot_be_forwarded(self):
        original = copy.deepcopy(self.row)
        for case in ('partial', 'stale', 'warning'):
            self.row = copy.deepcopy(original)
            if case == 'partial':
                self.row['result']['partial'] = True
            elif case == 'stale':
                self.row['stale'] = True
            else:
                self.row['result']['items'].append({'severity': 'warning', 'message': 'unresolved'})
            self.save_log()
            with self.assertRaises(ValueError):
                self.helper.read_messages(self.source, self.log, self.evidence)

    def test_full_component_must_match_exact_registered_names(self):
        record = dict(success=True, audits=1, files=[self.evidence])
        self.assertEqual(self.helper.component_messages(self.root, self.root, record, {'Example.bound'}),
                         [self.message])
        with self.assertRaisesRegex(ValueError, 'exact audit inventory'):
            self.helper.component_messages(self.root, self.root, record, {'Example.other'})


if __name__ == '__main__':
    unittest.main()
