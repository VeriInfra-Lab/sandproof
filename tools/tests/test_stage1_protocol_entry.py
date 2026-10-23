"""Fail-closed source/archive and retained-proof admission; no Lean or Rust build."""
from pathlib import Path
import hashlib
import importlib.util
import copy
import io
import json
import os
import shutil
import shlex
import sys
import subprocess
import tarfile
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT/'verification/experiments/stage1-protocol'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


archive = load('stage1_archive_under_test', HERE/'archive.py')
formal = load('stage1_formal_under_test', HERE/'formal.py')
component = load('stage1_component_under_test', HERE/'component.py')
extractor = load('stage1_extractor_under_test', HERE/'extract.py')


class CommittedArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base/'source'
        self.root.mkdir()
        subprocess.run(['git', 'init', '-q', self.root], check=True)
        for name, content in [('Cargo.toml', '[workspace]\n'), ('fixture.json', '{}\n'),
                              ('docs/contract.txt', 'all tracked inputs\n'), ('README.md', 'old report\n'),
                              ('scripts/entry.sh', '#!/bin/sh\nexit 0\n')]:
            file = self.root/name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(content)
        (self.root/'scripts/entry.sh').chmod(0o755)
        self.commit()

    def commit(self):
        subprocess.run(['git', 'add', '.'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                        'commit', '-qm', 'fixture'], cwd=self.root, check=True)

    def build(self, name='first', root=None):
        return archive.build(root or self.root, self.base/(name+'.tar'), self.base/(name+'.json'))

    def test_full_committed_contents_and_execute_bit_survive_location_and_mtime_changes(self):
        first = self.build()
        other = self.base/'second-source'
        shutil.copytree(self.root, other)
        for file in other.rglob('*'):
            if file.is_file() and '.git' not in file.relative_to(other).parts:
                os.utime(file, (123456789, 123456789))
        self.build('second', other)
        self.assertEqual((self.base/'first.tar').read_bytes(), (self.base/'second.tar').read_bytes())
        self.assertEqual((self.base/'first.json').read_bytes(), (self.base/'second.json').read_bytes())
        with tarfile.open(self.base/'first.tar') as tar:
            self.assertEqual(set(tar.getnames()), set(first['files']))
            self.assertEqual(tar.extractfile('fixture.json').read(), b'{}\n')
            self.assertEqual(tar.extractfile('docs/contract.txt').read(), b'all tracked inputs\n')
            self.assertEqual(tar.getmember('scripts/entry.sh').mode, 0o755)

    def test_dirty_tracked_file_and_untracked_wip_are_rejected_before_publication(self):
        for name in ('fixture.json', 'untracked-wip.rs'):
            with self.subTest(name=name):
                file = self.root/name
                before = file.read_bytes() if file.exists() else None
                file.write_text('changed')
                with self.assertRaises(ValueError):
                    self.build()
                self.assertFalse((self.base/'first.tar').exists())
                if before is None:
                    file.unlink()
                else:
                    file.write_bytes(before)

    def test_tracked_symlink_does_not_read_or_import_its_target(self):
        (self.root/'linked').symlink_to('fixture.json')
        self.commit()
        with self.assertRaises(ValueError):
            self.build()
        self.assertFalse((self.base/'first.json').exists())

    def test_source_change_during_archive_prevents_manifest_publication(self):
        original = archive.committed_inputs
        calls = 0

        def changed(root):
            nonlocal calls
            calls += 1
            if calls == 2:
                (root/'fixture.json').write_text('changed during archive')
            return original(root)

        with mock.patch.object(archive, 'committed_inputs', side_effect=changed):
            with self.assertRaises(ValueError):
                self.build()
        self.assertFalse((self.base/'first.json').exists())

    def test_stage1_key_reuses_reports_and_revision_but_binds_related_execute_bit(self):
        from tools import acceptance_receipts
        cache = self.base/'cache'
        cache.mkdir()
        (cache/'runtime.json').write_text('{"unit_test_fixture":true}')
        with mock.patch.dict(os.environ, {'SANDPROOF_NATIVE_EVIDENCE': str(cache)}):
            def key():
                return acceptance_receipts.native_key('stage1-protocol', ['component-case'],
                                                     self.root, root=self.root, environment={})
            before = key()
            (self.root/'README.md').write_text('changed docs, same Rust and rules\n')
            with self.assertRaises(ValueError):
                key()
            self.commit()
            after_docs = key()
            self.assertEqual(before['rust'], after_docs['rust'])
            self.assertEqual(before['rules'], after_docs['rules'])
            self.assertEqual(before, after_docs)
            (self.root/'scripts/entry.sh').chmod(0o644)
            self.commit()
            after_mode = key()
            self.assertEqual(after_docs['rules'], after_mode['rules'])
            self.assertNotEqual(after_docs, after_mode)
            subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                            'commit', '--allow-empty', '-qm', 'same tree candidate'],
                           cwd=self.root, check=True)
            self.assertEqual(after_mode, key())
            (self.root/'scripts/entry.sh').write_text('#!/bin/sh\nexit 1\n')
            self.commit()
            self.assertNotEqual(after_mode, key())

    def test_shell_entry_passes_a_new_child_not_the_existing_temporary_parent(self):
        # Exercise the real shell path and real component mkdir/archive path.
        # A forbidden role stops before any native/proof command; all other
        # shell consumers are explicitly synthetic fixtures, not acceptance.
        entry = self.root/'scripts/verify-stage1-protocol.sh'
        shutil.copy2(ROOT/'scripts/verify-stage1-protocol.sh', entry)
        (self.root/'verification').mkdir()
        digest = hashlib.sha256((self.root/'fixture.json').read_bytes()).hexdigest()
        (self.root/'verification/stage1-protocol-inputs.sha256').write_text(digest+'  fixture.json\n')
        runner = self.root/'entry-path-fixture.py'
        runner.write_text(
            'from pathlib import Path\nimport importlib.util,sys\n'
            + 'p=Path(' + repr(str(HERE/'component.py')) + ')\n'
            + 'spec=importlib.util.spec_from_file_location("entry_fixture",p)\n'
            + 'module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)\n'
            + 'module.ROOT=Path(' + repr(str(self.root)) + ')\n'
            + 'assert sys.argv[2]=="--output"\n'
            + 'result=module.run(Path(sys.argv[3]),True)\n'
            + 'assert result["complete"] is False and "native host role" in result["error"]\n'
            + 'print("entry-output-new")\n')
        python_entry = self.root/'scripts/python-entry.sh'
        python_entry.write_text('#!/bin/sh\n'
            + 'if [ "$1" = "tools/proof_native_suite.py" ]; then exit 0; fi\n'
            + 'exec ' + shlex.quote(sys.executable) + ' ' + shlex.quote(str(runner)) + ' "$@"\n')
        self.commit()
        tmp = self.base/'entry-tmp'
        tmp.mkdir()
        result = subprocess.run(['bash', str(entry)], cwd=self.root, text=True, capture_output=True,
            env=dict(os.environ, TMPDIR=str(tmp), SANDPROOF_ACCEPTANCE_ROLE='path-fixture-only'))
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn('entry-output-new', result.stdout)
        outputs = list(tmp.glob('sandproof-stage1-protocol.*/reproduction/component-result.json'))
        self.assertEqual(len(outputs), 1)
        self.assertFalse(json.loads(outputs[0].read_text())['complete'])

    def test_formal_wrapper_cannot_replace_missing_shared_native_seal_with_bare_success(self):
        from tools import acceptance_receipts
        cache = self.base/'missing-evidence'
        with mock.patch.object(component, 'ROOT', self.root), \
                mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'formal',
                                             'SANDPROOF_NATIVE_EVIDENCE': str(cache)}), \
                mock.patch.object(acceptance_receipts, 'validate_native',
                                  side_effect=ValueError('missing native seal')) as validate, \
                mock.patch.object(acceptance_receipts, 'native_case') as admit:
            result = component.run(self.base/'formal', False)
        validate.assert_called_once_with(cache)
        admit.assert_not_called()
        self.assertFalse(result['complete'])
        self.assertIn('missing native seal', result['error'])
        self.assertFalse((self.base/'formal/formal').exists())


class Stage1ReceiptReuseTests(unittest.TestCase):
    setUp = CommittedArchiveTests.setUp
    commit = CommittedArchiveTests.commit
    # Exercise the real case cache/publication with tiny, explicitly synthetic
    # archives. These fixtures never claim Rust/Lean/CI evidence.
    def test_docs_and_same_tree_revision_reuse_case_and_global_native_dependencies(self):
        from tools import acceptance_receipts, proof_batch
        cache = self.base/'native-evidence'
        cache.mkdir()
        runtime = {'unit_test_fixture': True}
        (cache/'runtime.json').write_text(json.dumps(runtime))
        calls = []

        def invoke(name):
            out = self.base/name

            def action():
                calls.append(name)
                out.mkdir()
                archive.build(self.root, out/'source.tar', out/'source-manifest.json')
                (out/'result.json').write_text('{"unit_test_fixture":true}')

            return acceptance_receipts.native_case('stage1-protocol', self.root, out, action)

        with mock.patch.dict(os.environ, {'SANDPROOF_NATIVE_EVIDENCE': str(cache),
                                         'SANDPROOF_ACCEPTANCE_ROLE': 'native',
                                         **{name: '' for name in acceptance_receipts.SOURCE_ARCHIVES}}):
            first = invoke('first-case')
            entry = cache/'commands'/acceptance_receipts.digest(first['key'])
            seal = dict(schema=acceptance_receipts.NATIVE_SCHEMA, rules=first['key']['rules'],
                        rust=first['key']['rust'], contracts=first['key']['contracts'],
                        source_archives=first['key']['source_archives'], runtime=runtime,
                        commands={entry.name: acceptance_receipts.sha(entry/'receipt.json')})
            (cache/'complete.json').write_text(json.dumps(seal))
            dependencies = proof_batch.native_dependencies(cache)
            historical = archive.verify(entry/'evidence/source.tar', entry/'evidence/source-manifest.json')
            (self.root/'README.md').write_text('new report only\n')
            self.commit()
            second = invoke('docs-case')
            self.assertEqual(first['key'], second['key'])
            subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                            'commit', '--allow-empty', '-qm', 'same input revision'],
                           cwd=self.root, check=True)
            invoke('same-tree-case')
            self.assertEqual(calls, ['first-case'])
            self.assertEqual(historical, archive.verify(entry/'evidence/source.tar', entry/'evidence/source-manifest.json'))
            self.assertEqual(dependencies, proof_batch.native_dependencies(cache))
            original = entry/'evidence/source.tar'
            original.write_bytes(original.read_bytes()+b'damaged')
            invoke('damaged-tar-case')
            self.assertEqual(calls, ['first-case', 'damaged-tar-case'])
            # Historical original and current archives can differ in reports;
            # neither original evidence nor its manifest is relabelled.
            current = archive.verify(entry/'evidence/source.tar', entry/'evidence/source-manifest.json')
            self.assertIn('docs/contract.txt', current['files'])

    def test_old_functional_rejects_current_readme_until_current_functional_is_provided(self):
        from tools import acceptance_receipts, proof_batch
        cache = self.base/'native-evidence'
        cache.mkdir()
        runtime = {'unit_test_fixture': True}
        (cache/'runtime.json').write_text(json.dumps(runtime))
        stage_calls = []

        def publish(label, name):
            out = self.base/name

            def action():
                out.mkdir()
                (out/'fixture.log').write_text('synthetic admission fixture, not actual CI\n')
                if label == 'stage1-protocol':
                    stage_calls.append(name)
                    archive.build(self.root, out/'source.tar', out/'source-manifest.json')

            return acceptance_receipts.native_case(label, self.root, out, action)

        def seal(functional, stage):
            commands = {}
            for receipt in (functional, stage):
                entry = cache/'commands'/acceptance_receipts.digest(receipt['key'])
                commands[entry.name] = acceptance_receipts.sha(entry/'receipt.json')
            record = dict(schema=acceptance_receipts.NATIVE_SCHEMA, complete=True, exit_code=0,
                rules=stage['key']['rules'], rust=stage['key']['rust'], runtime=runtime,
                contracts=stage['key']['contracts']['review'],
                source_archives=stage['key']['source_archives'], commands=commands)
            (cache/'complete.json').write_text(json.dumps(record))

        with mock.patch.dict(os.environ, {'SANDPROOF_NATIVE_EVIDENCE': str(cache),
                                         'SANDPROOF_ACCEPTANCE_ROLE': 'native',
                                         **{name: '' for name in acceptance_receipts.SOURCE_ARCHIVES}}):
            stage = publish('stage1-protocol', 'stage')
            old_functional = publish('functional', 'old-functional')
            seal(old_functional, stage)
            before = proof_batch.native_dependencies(cache)
            (self.root/'README.md').write_text('new report, unchanged native inputs\n')
            self.commit()
            os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
            # Valid old logs alone are insufficient to admit current docs.
            self.assertTrue(acceptance_receipts.validate_native(cache)['complete'])
            with self.assertRaises(ValueError):
                component.admit_native(cache, self.root)
            os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'native'
            current_functional = publish('functional', 'current-functional')
            seal(current_functional, stage)
            os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
            admitted = component.admit_native(cache, self.root)
            self.assertEqual(admitted['key'], stage['key'])
            self.assertEqual(stage_calls, ['stage'])
            self.assertEqual(before, proof_batch.native_dependencies(cache))

    def test_missing_or_linked_required_tar_never_publishes_a_case(self):
        from tools import acceptance_receipts
        cache = self.base/'native-evidence'
        cache.mkdir()
        (cache/'runtime.json').write_text('{"unit_test_fixture":true}')
        with mock.patch.dict(os.environ, {'SANDPROOF_NATIVE_EVIDENCE': str(cache),
                                         'SANDPROOF_ACCEPTANCE_ROLE': 'native',
                                         **{name: '' for name in acceptance_receipts.SOURCE_ARCHIVES}}):
            for name in ('missing', 'linked'):
                out = self.base/name

                def action():
                    out.mkdir()
                    (out/'result.json').write_text('{"unit_test_fixture":true}')
                    if name == 'linked':
                        (out/'source.tar').symlink_to(self.root/'Cargo.toml')

                with self.subTest(name=name), self.assertRaises(ValueError):
                    acceptance_receipts.native_case('stage1-protocol', self.root, out, action)
                key = acceptance_receipts.native_key('stage1-protocol', ['component-case'], self.root)
                entry = cache/'commands'/acceptance_receipts.digest(key)
                self.assertFalse((entry/'receipt.json').exists())


class NativeSourceReconstructionTests(unittest.TestCase):
    def test_missing_transported_rs_is_rebuilt_outside_immutable_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            evidence = root/'sealed-native'
            evidence.mkdir()
            (evidence/'result.json').write_text('{"success":true}')
            original = b'before unique expression\n'
            mutation = dict(source='sandbox-protocol/src/example.rs', old='unique expression', new='wrong')
            wrong = b'before wrong\n'
            row = dict(original_source_sha256=hashlib.sha256(original).hexdigest(),
                       wrong_source_sha256=hashlib.sha256(wrong).hexdigest())
            file = root/'source.tar'
            with tarfile.open(file, 'x') as tar:
                entry = tarfile.TarInfo(mutation['source'])
                entry.size = len(original)
                tar.addfile(entry, io.BytesIO(original))
            before = {p.name: p.read_bytes() for p in evidence.iterdir()}
            with tarfile.open(file) as tar:
                result = formal.reconstruct_native_source(tar, mutation, row, root/'formal-work/variant')
            self.assertEqual(Path(result['file']).read_bytes(), wrong)
            self.assertEqual(result['sha256'], row['wrong_source_sha256'])
            self.assertEqual(before, {p.name: p.read_bytes() for p in evidence.iterdir()})
            self.assertFalse((evidence/'mutation.rs').exists())
            self.assertIn('reconstruction', result['kind'])

    def test_changed_wrong_digest_and_ambiguous_mutation_never_publish_source(self):
        for original, expected_wrong in ((b'unique', '0'*64), (b'unique unique', '0'*64)):
            with self.subTest(original=original), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                file = root/'source.tar'
                mutation = dict(source='src.rs', old='unique', new='wrong')
                row = dict(original_source_sha256=hashlib.sha256(original).hexdigest(),
                           wrong_source_sha256=expected_wrong)
                with tarfile.open(file, 'x') as tar:
                    entry = tarfile.TarInfo('src.rs')
                    entry.size = len(original)
                    tar.addfile(entry, io.BytesIO(original))
                with tarfile.open(file) as tar, self.assertRaises(ValueError):
                    formal.reconstruct_native_source(tar, mutation, row, root/'formal-work')
                self.assertFalse((root/'formal-work/mutation.rs').exists())


class NativePhaseAdmissionTests(unittest.TestCase):
    def test_stale_library_foreign_manifest_and_wrong_program_are_not_native_counterexamples(self):
        mutation = dict(target='current_envelope', test_source='sandbox-protocol/tests/current_envelope.rs',
                        test='original_assertion')
        artifacts = [
            dict(reason='compiler-artifact', fresh=False, manifest_path='/ordinary/source/sandbox-protocol/Cargo.toml',
                 target=dict(name='sandproof_protocol', kind=['lib'],
                             src_path='/ordinary/source/sandbox-protocol/src/lib.rs'),
                 profile=dict(test=False), executable=None),
            dict(reason='compiler-artifact', fresh=False, manifest_path='/ordinary/source/sandbox-protocol/Cargo.toml',
                 target=dict(name='current_envelope', kind=['test'],
                             src_path='/ordinary/source/sandbox-protocol/tests/current_envelope.rs'),
                 profile=dict(test=True), executable='/ordinary/target/current_envelope-binary')]
        for failure in (None, 'stale-library', 'foreign-manifest', 'foreign-program'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                folder = Path(temp)
                actual = copy.deepcopy(artifacts)
                if failure == 'stale-library':
                    actual[0]['fresh'] = True
                if failure == 'foreign-manifest':
                    actual[0]['manifest_path'] = '/other/sandbox-protocol/Cargo.toml'
                build = folder/'build.log'
                build.write_text(''.join(json.dumps(item)+'\n' for item in actual))
                assertion = folder/'assertions.log'
                assertion.write_text('assertion failed\noriginal_assertion\n'
                                     'test result: FAILED. 0 passed; 1 failed; 0 ignored;\n')
                argv = [actual[1]['executable'], mutation['test'], '--exact', '--nocapture', '--test-threads=1']
                if failure == 'foreign-program':
                    argv[0] = '/other/program'
                row = dict(build=dict(exit_code=0, log_sha256=formal.sha(build)),
                           assertion=dict(exit_code=101, log_sha256=formal.sha(assertion), argv=argv),
                           compiler_artifacts=actual, compiled_sha256={'current_envelope-binary': '0'*64})
                if failure is None:
                    formal.check_native_phase(folder, row, mutation, 'build', 'assertion', True)
                else:
                    with self.assertRaises(ValueError):
                        formal.check_native_phase(folder, row, mutation, 'build', 'assertion', True)


class ExtractionControlFlowTests(unittest.TestCase):
    def test_compile_log_check_keeps_environment_dictionary_and_rejects_foreign_target(self):
        # Synthetic compiler output only; deliberately fail before any Lean
        # generation. This tests admission/control flow, never tool validity.
        for target in ('fixture-target', 'foreign-target'):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                tool_root = root/'tools'
                environment = {'target': 'fixture-target'}
                for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
                    binary = tool_root/relative
                    binary.parent.mkdir(parents=True, exist_ok=True)
                    binary.write_bytes(b'synthetic-not-a-tool')
                    environment[name] = {'binary_sha256': extractor.sha(binary)}
                for path, name in [('sandbox-protocol', 'sandproof-protocol'),
                                   ('verification/experiments/stage1-protocol', 'sandproof-stage1-protocol-extract')]:
                    manifest = root/path/'Cargo.toml'
                    manifest.parent.mkdir(parents=True, exist_ok=True)
                    manifest.write_text('[package]\nname="'+name+'"\nversion="0.1.0"\n')
                config = root/'verification/foundations-environment.json'
                config.write_text(json.dumps(environment))
                calls = []

                def compiler(argv, **kwargs):
                    name = Path(argv[0]).name
                    calls.append(name)
                    if name == 'charon':
                        text = ('   Compiling sandproof-protocol v0.1.0 ('+str(root/'sandbox-protocol')+')\n'
                                '   Compiling sandproof-stage1-protocol-extract v0.1.0 ('
                                +str(root/'verification/experiments/stage1-protocol')+')\n')
                        kwargs['stdout'].write(text.encode())
                        raw = root/'artifacts/stage1-protocol/raw/request.llbc'
                        raw.write_text(json.dumps({'has_errors': False, 'translated': {
                            'options': {'dest_file': '../../../artifacts/stage1-protocol/raw/request.llbc'},
                            'target_information': [{'key': target}]}}))
                        return types.SimpleNamespace(returncode=0)
                    return types.SimpleNamespace(returncode=1)  # Never generate or prove.

                digest = types.ModuleType('check_foundations_extraction')
                digest.llbc_digest = lambda _: '0'*64
                with mock.patch.object(extractor, 'ROOT', root), \
                        mock.patch.object(extractor, 'inputs', return_value={}), \
                        mock.patch.object(extractor.subprocess, 'run', side_effect=compiler), \
                        mock.patch.dict(sys.modules, {'check_foundations_extraction': digest}):
                    result = extractor.extract(root/'output', ['request'], tool_root)
                self.assertFalse(result['success'])
                if target == 'fixture-target':
                    self.assertEqual(calls, ['charon', 'aeneas'])
                    self.assertIn('Aeneas failed', result['error'])
                else:
                    self.assertEqual(calls, ['charon'])
                    self.assertIn('raw target does not match', result['error'])
                self.assertNotIn('TypeError', result['error'])


class ProtocolInventoryTests(unittest.TestCase):
    def test_every_retained_proof_must_be_registered_and_none_can_disappear(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            directory = root/formal.PROOFS
            directory.mkdir(parents=True)
            for name in formal.POSITIVE:
                (directory/(name+'.lean')).write_text('')
            frozen = {'positive_files': formal.POSITIVE_BY_PROFILE}
            formal.check_inventory(root, frozen)
            (directory/'Unregistered.lean').write_text('')
            with self.assertRaises(ValueError):
                formal.check_inventory(root, frozen)
            (directory/'Unregistered.lean').unlink()
            (directory/'Recorded.lean').unlink()
            with self.assertRaises(ValueError):
                formal.check_inventory(root, frozen)


if __name__ == '__main__':
    unittest.main()
