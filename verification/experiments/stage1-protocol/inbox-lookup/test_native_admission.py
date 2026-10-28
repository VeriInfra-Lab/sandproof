"""Relocation must not admit changed sources, locks, or registry contents."""
from copy import deepcopy
from hashlib import sha256
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('inbox_native_admission_test',
    Path(__file__).with_name('native_admission.py'))
admission = importlib.util.module_from_spec(spec)
spec.loader.exec_module(admission)


class NativeAdmission(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory(prefix='inbox-admission-')
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        self.lock = Path('Cargo.lock')
        text = ('version = 4\n[[package]]\nname = "crate-a"\nversion = "1.0.0"\n'
                'source = "registry+https://example.invalid/index"\n'
                'checksum = "' + 'a' * 64 + '"\n')
        (self.root / self.lock).write_text(text)
        self.inputs = {'Cargo.lock': sha256(text.encode()).hexdigest(), 'formal.py': 'f' * 64}
        self.native = dict(inputs=deepcopy(self.inputs), registry=self.registry('/native/cargo'))
        self.current = self.registry('/formal/cargo')
        self.record = {}

    @staticmethod
    def registry(root):
        return dict(archives={root + '/registry/cache/provider/crate-a-1.0.0.crate': 'a' * 64},
                    files={root + '/registry/src/provider/crate-a-1.0.0/src/lib.rs': 'b' * 64,
                           root + '/registry/src/provider/crate-a-1.0.0/.cargo-ok': 'c' * 64})

    def admit(self):
        admission.admit(self.record, self.native, self.inputs, self.current,
                        self.root, self.lock)

    def test_relocation_keeps_both_raw_records_and_source_check(self):
        before = deepcopy(self.native)
        self.admit()
        self.assertTrue(self.record['native_inputs_match'])
        self.assertTrue(self.record['registry_content_match'])
        self.assertEqual(self.record['native_registry'], before['registry'])
        self.assertEqual(self.record['registry'], self.current)
        self.assertEqual(self.native, before)

    def test_new_checker_source_still_requires_new_input_admission(self):
        self.inputs['formal.py'] = '0' * 64
        before = deepcopy(self.native)
        with self.assertRaisesRegex(ValueError, 'source inputs differ'):
            self.admit()
        self.assertEqual(self.native, before)
        self.assertNotIn('registry_content_match', self.record)
        self.assertEqual(self.record['native_registry'], before['registry'])

    def test_missing_source_is_rejected(self):
        del self.inputs['formal.py']
        with self.assertRaisesRegex(ValueError, 'source inputs differ'):
            self.admit()

    def test_actual_lock_bytes_must_match_source_snapshot(self):
        with (self.root / self.lock).open('a') as stream:
            stream.write('# changed after snapshot\n')
        with self.assertRaisesRegex(ValueError, 'Cargo.lock differs'):
            self.admit()

    def test_same_locations_do_not_hide_changed_source_bytes(self):
        self.current = deepcopy(self.native['registry'])
        self.current['files'][next(iter(self.current['files']))] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'contents differ'):
            self.admit()

    def test_development_without_native_is_not_labeled_admitted(self):
        self.native = None
        self.admit()
        self.assertNotIn('native_inputs_match', self.record)
        self.assertNotIn('registry_content_match', self.record)


if __name__ == '__main__':
    unittest.main()
