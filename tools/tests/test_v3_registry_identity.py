"""Cross-machine location changes must not hide changed Cargo contents."""
from copy import deepcopy
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('v3_registry_identity',
    ROOT / 'verification/experiments/stage1-protocol/registry_identity.py')
registry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registry)


class RegistryIdentity(unittest.TestCase):
    def setUp(self):
        self.lock = {'package': [dict(name='crate-a', version='1.0.0',
            source='registry+https://example.invalid/index', checksum='a' * 64)]}
        self.left = self.evidence('/native/cargo')
        self.right = self.evidence('/formal/cargo')

    @staticmethod
    def evidence(root):
        return dict(archives={root + '/registry/cache/provider/crate-a-1.0.0.crate': 'a' * 64},
                    files={root + '/registry/src/provider/crate-a-1.0.0/src/lib.rs': 'b' * 64,
                           root + '/registry/src/provider/crate-a-1.0.0/.cargo-ok': 'c' * 64})

    def test_relocation_preserves_content_and_raw_records(self):
        before = deepcopy((self.left, self.right))
        registry.require_same_content(self.left, self.right, self.lock)
        self.assertEqual((self.left, self.right), before)

    def test_changed_archive_rejected(self):
        self.right['archives'][next(iter(self.right['archives']))] = 'd' * 64
        with self.assertRaisesRegex(ValueError, 'archive differs'):
            registry.require_same_content(self.left, self.right, self.lock)

    def test_changed_or_missing_source_rejected(self):
        for remove in (False, True):
            right = deepcopy(self.right)
            path = next(iter(right['files']))
            if remove:
                del right['files'][path]
            else:
                right['files'][path] = 'd' * 64
            with self.subTest(remove=remove), self.assertRaisesRegex(ValueError, 'contents differ'):
                registry.require_same_content(self.left, right, self.lock)

    def test_extra_source_or_metadata_rejected(self):
        for path in ('src/extra.rs', '.cargo-checksum.json'):
            right = deepcopy(self.right)
            right['files']['/formal/cargo/registry/src/provider/crate-a-1.0.0/' + path] = 'd' * 64
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'contents differ'):
                registry.require_same_content(self.left, right, self.lock)

    def test_missing_package_and_orphan_source_rejected(self):
        with self.assertRaisesRegex(ValueError, 'missing locked'):
            registry.content_identity(dict(archives={}, files={}), self.lock)
        right = deepcopy(self.right)
        right['files']['/other/cargo/registry/src/provider/crate-a-1.0.0/lib.rs'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'no locked archive'):
            registry.content_identity(right, self.lock)

    def test_duplicate_provider_count_is_preserved(self):
        extra = self.evidence('/extra/cargo')
        self.right['archives'].update(extra['archives'])
        self.right['files'].update(extra['files'])
        with self.assertRaisesRegex(ValueError, 'contents differ'):
            registry.require_same_content(self.left, self.right, self.lock)

    def test_unsafe_paths_and_ambiguous_lock_rejected(self):
        right = deepcopy(self.right)
        right['files']['/formal/cargo/registry/src/provider/crate-a-1.0.0/../x'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'non-canonical'):
            registry.content_identity(right, self.lock)
        self.lock['package'].append(dict(self.lock['package'][0]))
        with self.assertRaisesRegex(ValueError, 'ambiguous locked'):
            registry.content_identity(self.left, self.lock)


if __name__ == '__main__':
    unittest.main()
