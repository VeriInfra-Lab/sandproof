"""Source inventory admission before Charon, Cargo, or a variant rewrite."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import admit


class ProductionInputInventory(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.contents = {
            'Cargo.lock': b'lock fixture',
            'controller-service/Cargo.toml': b'manifest fixture',
            'controller-service/src/lib.rs': b'Rust fixture',
        }
        self.profile = {
            'production_inputs': {name: hashlib.sha256(data).hexdigest()
                                  for name, data in self.contents.items()},
        }
        for name, data in self.contents.items():
            self.write(name, data)

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_archive_accepts_exact_reviewed_inputs(self):
        self.write('verification/experiment/helper.rs', b'proof-only fixture')
        self.write('README.md', b'documentation fixture')
        admit.verify_archived_inputs(self.root, self.profile)

    def test_archive_rejects_extra_and_missing_source_paths(self):
        extra = self.write('controller-service/src/new.rs', b'new fixture')
        with self.assertRaisesRegex(ValueError, r"extra=\['controller-service/src/new.rs'\]"):
            admit.verify_archived_inputs(self.root, self.profile)
        extra.unlink()
        (self.root / 'Cargo.lock').unlink()
        with self.assertRaisesRegex(ValueError, r"missing=\['Cargo.lock'\]"):
            admit.verify_archived_inputs(self.root, self.profile)

    def test_archive_rejects_changed_bytes_and_same_bytes_through_a_symlink(self):
        name = 'controller-service/src/lib.rs'
        path = self.write(name, b'different Rust fixture')
        with self.assertRaisesRegex(ValueError, 'reviewed production input changed: ' + name):
            admit.verify_archived_inputs(self.root, self.profile)

        self.write('verification/experiment/library.rs', self.contents[name])
        path.unlink()
        path.symlink_to('../../verification/experiment/library.rs')
        with self.assertRaisesRegex(ValueError, 'reviewed production input changed: ' + name):
            admit.verify_archived_inputs(self.root, self.profile)

    def test_tracked_extra_refuses_before_inspecting_external_libraries(self):
        head = '7' * 40
        tracked = '\0'.join((*self.contents, 'controller-service/src/new.rs', '')).encode()
        with patch.object(admit, 'ROOT', self.root), \
                patch.object(admit.subprocess, 'check_output', return_value=tracked) as command, \
                patch.object(admit.importlib.util, 'spec_from_file_location') as library:
            with self.assertRaisesRegex(ValueError, 'production input inventory changed'):
                admit.inputs(self.profile, head)
            command.assert_called_once_with(
                ['git', 'ls-tree', '-r', '--name-only', '-z', head], cwd=self.root)
            library.assert_not_called()


if __name__ == '__main__':
    unittest.main()
