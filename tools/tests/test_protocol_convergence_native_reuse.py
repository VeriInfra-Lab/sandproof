"""Small admission regressions; synthetic receipts are never proof evidence."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts
from tools import proof_native_suite

PATH = receipts.ROOT/'verification/experiments/stage1-protocol/convergence.py'
SPEC = importlib.util.spec_from_file_location('bounded_native_reuse_test', PATH)
convergence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(convergence)
SCOPE = 'stage1-protocol-convergence'
LABEL = 'stage1-standard-digest'


class FixedNativeReuseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.cache = self.root/'evidence'
        self.cache.mkdir()
        manifest = self.root/convergence.NATIVE_REUSE_MANIFEST
        manifest.parent.mkdir(parents=True)
        manifest.write_text('unit receipt fixture, not acceptance evidence')
        own = 'verification/experiments/stage1-protocol/standard-digest/native.py'
        self.key = dict(schema=2, label=LABEL, command=['component-case'], options={},
            rust={'service.rs':'same'}, contracts={}, runtime={'fixture':True},
            source_archives={}, rules={own:'new-presence', 'unrelated/fixture.rs.in':'new'})
        old = copy.deepcopy(self.key)
        old['rules'][own] = 'old'
        old['rules']['unrelated/fixture.rs.in'] = 'old'
        self.original = dict(complete=True, exit_code=0, key=old, seconds=123,
                             files={'original.log':receipts.digest('original test outcomes')})
        self.identity = receipts.digest(old)
        self.entry = self.cache/'commands'/self.identity
        self.entry.mkdir(parents=True)
        (self.entry/'original.log').write_text('original test outcomes')
        self.original['files']['original.log'] = receipts.sha(self.entry/'original.log')
        receipts.write_json(self.entry/'receipt.json', self.original)
        self.original_bytes = (self.entry/'receipt.json').read_bytes()
        self.plan = dict(identity=self.identity,
            receipt_sha256=receipts.sha(self.entry/'receipt.json'), execution_commit='a'*40,
            component_prefixes=['verification/experiments/stage1-protocol/standard-digest/'],
            dependency_files=[], current_component_rules_sha256=receipts.digest({own:'new-presence'}))
        self.definition = dict(components={LABEL:self.plan}, admission_rules={},
            shared_rules_sha256=receipts.digest({'unrelated/fixture.rs.in':'new'}))
        # The unrelated component is excluded by the real finite prefix rule.
        self.key['rules'].pop('unrelated/fixture.rs.in')
        self.key['rules']['verification/experiments/stage1-protocol/record-reply/native-cases.rs.in'] = 'changed-other-component'
        self.definition['shared_rules_sha256'] = receipts.digest({})
        self.output = self.root/'new-component'
        patches = [
            mock.patch.object(receipts, 'ROOT', self.root),
            mock.patch.object(receipts, 'stage1_registration', return_value=types.SimpleNamespace(CONVERGENCE=convergence)),
            mock.patch.object(receipts, 'native_key', side_effect=lambda label,*a,**k:dict(self.key,label=label)),
            mock.patch.object(receipts, 'scoped_native_labels', return_value=(LABEL,)),
            mock.patch.object(convergence, 'native_reuse_definition', return_value=self.definition),
            mock.patch.dict(os.environ, SANDPROOF_NATIVE_EVIDENCE=str(self.cache),
                SANDPROOF_ACCEPTANCE_ROLE='native', SANDPROOF_CONVERGENCE_REUSE_FIXED17='1'),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def reuse(self):
        return receipts.native_case(LABEL, self.root, self.output,
            lambda: self.fail('unchanged original component was executed'), scope=SCOPE)

    def test_original_identity_and_bytes_survive_unrelated_fixture_change(self):
        result = self.reuse()
        self.assertEqual(result, self.original)
        self.assertEqual((self.entry/'receipt.json').read_bytes(), self.original_bytes)
        keys = {self.identity:self.original['key']}
        references = receipts.convergence_reuse_publication(self.cache, keys, scope=SCOPE)
        self.assertEqual(references[LABEL]['identity'], self.identity)
        self.assertEqual(json.loads((self.cache/references[LABEL]['file']).read_text())['current_key_sha256'], receipts.digest(self.key))

    def test_formal_consumer_checks_the_same_admission_and_original(self):
        self.reuse()
        references = receipts.convergence_reuse_publication(self.cache,
            {self.identity:self.original['key']}, scope=SCOPE)
        seal = dict(scope=SCOPE, complete=True, commands={self.identity:receipts.sha(self.entry/'receipt.json')},
                    convergence_reuse=references)
        receipts.write_json(self.cache/'complete.json', seal)
        receipts.write_json(self.cache/'runtime.json', self.key['runtime'])
        os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'formal'
        os.environ.pop('SANDPROOF_CONVERGENCE_REUSE_FIXED17')
        with mock.patch.object(receipts, 'current_native_seal', return_value=True):
            self.assertEqual(receipts.native_case(LABEL, self.root, None, scope=SCOPE), self.original)
            seal.pop('convergence_reuse')
            receipts.write_json(self.cache/'complete.json', seal)
            with self.assertRaisesRegex(ValueError, 'no current admission'):
                receipts.validate_native(self.cache, scope=SCOPE)

    def test_changed_own_fixture_refuses_only_that_component_reuse(self):
        path = self.plan['component_prefixes'][0]+'native-cases.rs.in'
        self.key['rules'][path] = 'changed'
        called = []
        def action():
            called.append(True)
            self.output.mkdir()
            (self.output/'actual.log').write_text('new synthetic component action')
        result = receipts.native_case(LABEL, self.root, self.output, action, scope=SCOPE)
        self.assertEqual(called, [True])
        self.assertEqual(result['key'], self.key)
        self.assertEqual((self.entry/'receipt.json').read_bytes(), self.original_bytes)

    def test_corrupt_original_never_silently_executes_or_admits(self):
        (self.entry/'original.log').write_text('damaged')
        with self.assertRaisesRegex(ValueError, 'incomplete fixed original'):
            self.reuse()

    def test_environment_and_dependency_changes_refuse_reuse(self):
        for field in ('rust', 'options', 'runtime', 'contracts', 'source_archives'):
            changed = copy.deepcopy(self.key)
            changed[field]['different'] = 'new'
            with self.subTest(field=field):
                with self.assertRaisesRegex(ValueError, 'execution context changed'):
                    convergence.native_reuse_matches(self.root,LABEL,
                        self.original['key'],changed,receipts)

    def test_changed_current_admission_and_other_scope_are_rejected(self):
        self.reuse()
        references = receipts.convergence_reuse_publication(self.cache,
            {self.identity:self.original['key']}, scope=SCOPE)
        path = self.cache/references[LABEL]['file']
        value = json.loads(path.read_text())
        value['current_key_sha256'] = '0'*64
        receipts.write_json(path, value)
        with self.assertRaisesRegex(ValueError, 'changed current'):
            receipts.convergence_reuse_case(self.cache,LABEL,self.key,reference=references[LABEL])
        with self.assertRaisesRegex(ValueError, 'exact native scope'):
            proof_native_suite.run_locked_suite(self.root/'forbidden', scope='capture-caller')


class FixedDefinitionTests(unittest.TestCase):
    def test_production_definition_requires_exact_registered_originals(self):
        data = convergence.native_reuse_definition(receipts.ROOT)
        self.assertEqual(len(data['components']), 18)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root/convergence.NATIVE_REUSE_MANIFEST
            path.parent.mkdir(parents=True)
            data['components'].pop(next(iter(data['components'])))
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'unknown fixed'):
                convergence.native_reuse_definition(root)


class OriginalReplayDiagnosticTests(unittest.TestCase):
    """Synthetic diagnostic fragments, never a Rust counterexample receipt."""
    @classmethod
    def setUpClass(cls):
        path = PATH.parent/'protocol-record/native.py'
        spec = importlib.util.spec_from_file_location('original_replay_lint_test', path)
        cls.reader = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.reader)

    def test_only_original_replay_site_can_register_lint_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'recording.rs'
            source.write_text('synthetic unit source')
            expected = dict(code='dead_code', message='variant `Replay` is never constructed',
                source='controller-service/src/controller/lifecycle_inbox/recording.rs',
                line=17, column=5, source_sha256=receipts.sha(source), primary_count=1,
                cargo_errors=['error: could not compile `sandproof-controller-service` (lib) due to 1 previous error',
                              'error: could not compile `sandproof-controller-service` (lib test) due to 1 previous error'])
            text = ('error: '+expected['message']+'\n  --> '+expected['source']+':17:5\n'
                    '= note: `-D dead-code` implied by `-D warnings`\n'
                    +'\n'.join(expected['cargo_errors']))
            row = dict(exit_code=101)
            self.reader.check_clippy(row,text,source,'replay-falls-through',dict(native_replay_lint=expected))
            self.assertTrue(row['expected_mutant_lint_rejection'])
            self.assertNotIn('property_rejected', row)
            for bad in (text.replace(':17:5', ':18:5'), text+'\nerror: another failure',
                        text.replace('variant `Replay`', 'variant `Other`')):
                with self.subTest(text=bad), self.assertRaisesRegex(ValueError, 'unregistered'):
                    self.reader.check_clippy(dict(exit_code=101),bad,source,'replay-falls-through',
                                             dict(native_replay_lint=expected))
            with self.assertRaisesRegex(ValueError, 'clippy failed'):
                self.reader.check_clippy(dict(exit_code=101),text,source,'original',{})
