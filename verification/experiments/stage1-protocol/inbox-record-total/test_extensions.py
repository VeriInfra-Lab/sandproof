"""Additional proof admission must preserve the original checked closure."""
from pathlib import Path
import importlib.util
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('total_extension_checker', HERE/'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)
parent = formal.parent


class Extensions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='total-proof-extensions-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)/'root'
        self.project = Path(self.temp.name)/'project'
        self.root.mkdir()
        (self.project/'SandboxModel/Lifecycle/Protocol').mkdir(parents=True)
        self.source = Path('NewProof.lean.in')
        (self.root/self.source).write_text('-- proof input\n')
        self.inputs = {str(self.source): parent.sha(self.root/self.source)}
        self.expected = {'original.rs': 'unchanged'}
        self.stems = ['Original']

    def admit(self, sources):
        return parent.add_proofs(self.root, self.project, self.inputs,
                                 self.expected, self.stems, sources)

    def test_valid_addition_keeps_existing_roots_and_bytes(self):
        result = self.admit([self.source])
        target = 'SandboxModel/Lifecycle/Protocol/NewProof.lean'
        self.assertEqual(self.expected['original.rs'], 'unchanged')
        self.assertEqual(self.stems, ['Original', 'NewProof'])
        self.assertEqual(result[target]['sha256'], self.inputs[str(self.source)])
        self.assertEqual((self.project/target).read_bytes(), (self.root/self.source).read_bytes())

    def test_empty_extension_preserves_original_behavior(self):
        self.assertEqual(self.admit([]), {})
        self.assertEqual(self.expected, {'original.rs': 'unchanged'})
        self.assertEqual(self.stems, ['Original'])

    def test_existing_file_is_never_replaced(self):
        target = self.project/'SandboxModel/Lifecycle/Protocol/NewProof.lean'
        target.write_text('original proof')
        with self.assertRaisesRegex(ValueError, 'must not replace'):
            self.admit([self.source])
        self.assertEqual(target.read_text(), 'original proof')

    def test_duplicate_destination_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'must not replace'):
            self.admit([self.source, self.source])

    def test_source_outside_input_closure_is_rejected(self):
        self.inputs.clear()
        with self.assertRaisesRegex(ValueError, 'outside the checked input'):
            self.admit([self.source])

    def test_changed_source_is_not_admitted(self):
        (self.root/self.source).write_text('changed after snapshot')
        with self.assertRaisesRegex(ValueError, 'changed while copying'):
            self.admit([self.source])

    def test_absolute_parent_and_non_template_paths_are_rejected(self):
        for path in [self.root/self.source, Path('../NewProof.lean.in'), Path('NewProof.lean')]:
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'invalid additional'):
                self.admit([path])

    def test_invalid_module_name_is_rejected(self):
        self.inputs['1Bad.lean.in'] = self.inputs[str(self.source)]
        with self.assertRaisesRegex(ValueError, 'invalid additional proof module'):
            self.admit([Path('1Bad.lean.in')])


if __name__ == '__main__':
    unittest.main(verbosity=2)
