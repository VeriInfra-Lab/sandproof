"""Source revision tests without installing or starting the MCP transport."""
import importlib
from pathlib import Path
import sys
import tempfile
from types import ModuleType
import unittest
from unittest.mock import patch


class LeanSessionRevisionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Import the real public module; only its unused transport dependency is
        # replaced. Do not copy the fingerprint implementation into this test.
        transport = ModuleType('mcp')
        transport.ClientSession = object
        transport.StdioServerParameters = object
        transport.stdio_client = object
        with patch.dict(sys.modules, {'mcp': transport}):
            cls.client = importlib.import_module('tools.lean_session')

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.files = {
            'SandboxModel/Selected.lean': 'namespace Selected\nend Selected\n',
            'SandboxModel/Network/Policy.lean': 'namespace Policy\nend Policy\n',
            'verification/aeneas/first-chain/SandproofFirstChain/Funs.lean':
                'namespace FirstChain\nend FirstChain\n',
            'verification/aeneas/foundations/SandproofFoundations/Funs.lean':
                'namespace Foundations\nend Foundations\n',
            'verification/aeneas/network/SandproofNetwork/Funs.lean': 'namespace Network\nend Network\n',
            'verification/aeneas/network/SandproofNetwork/TypesExternal.lean':
                'namespace NetworkTypes\nend NetworkTypes\n',
            'lakefile.toml': 'name = "revision-test"\n',
            'lake-manifest.json': '{"packages": []}\n',
            'lean-toolchain': 'leanprover/lean4:v4.31.0\n',
            'artifacts/trial.lean': '#check Nat\n',
        }
        self.write_files(self.root, self.files)
        root_patch = patch.object(self.client, 'ROOT', self.root)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        versions_patch = patch.object(self.client.importlib.metadata, 'version',
                                      side_effect=lambda package: f'fixed-{package}')
        versions_patch.start()
        self.addCleanup(versions_patch.stop)

    @staticmethod
    def write_files(root, files):
        for name, content in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)

    def revision(self, path='SandboxModel/Selected.lean'):
        return self.client.revision({'file_path': path})

    def test_each_extraction_directory_and_handwritten_dependency_is_included(self):
        before = self.revision()
        for name in self.files:
            if not name.endswith('.lean') or name.startswith('artifacts/'):
                continue
            with self.subTest(path=name):
                source = self.root / name
                original = source.read_text()
                source.write_text(original + '-- changed dependency\n')
                after = self.revision()
                self.assertNotEqual(before, after)
                self.assertNotEqual(before['sources_sha256'], after['sources_sha256'])
                if name != 'SandboxModel/Selected.lean':
                    self.assertEqual(before['file_sha256'], after['file_sha256'])
                source.write_text(original)
                self.assertEqual(before, self.revision())

    def test_lakefile_change_marks_same_lean_file_as_stale(self):
        before = self.revision()
        (self.root / 'lakefile.toml').write_text('name = "revision-test-changed"\n')
        after = self.revision()
        self.assertNotEqual(before, after)
        self.assertNotEqual(before['lakefile_sha256'], after['lakefile_sha256'])
        self.assertEqual(before['sources_sha256'], after['sources_sha256'])
        self.assertEqual(before['file_sha256'], after['file_sha256'])

    def test_dependency_manifest_and_toolchain_are_versioned(self):
        for name, field in (('lake-manifest.json', 'manifest_sha256'), ('lean-toolchain', 'toolchain')):
            with self.subTest(path=name):
                before = self.revision()
                original = (self.root / name).read_text()
                (self.root / name).write_text(original + 'changed\n')
                after = self.revision()
                self.assertNotEqual(before, after)
                self.assertNotEqual(before[field], after[field])
                (self.root / name).write_text(original)

    def test_new_lean_file_under_each_library_is_not_omitted(self):
        for directory in ('SandboxModel/Network', 'verification/aeneas/first-chain',
                          'verification/aeneas/foundations', 'verification/aeneas/network'):
            with self.subTest(directory=directory):
                before = self.revision()
                path = self.root / directory / 'Nested/NewDependency.lean'
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('namespace NewDependency\nend NewDependency\n')
                self.assertNotEqual(before['sources_sha256'], self.revision()['sources_sha256'])
                path.unlink()
                self.assertEqual(before, self.revision())

    def test_removing_and_renaming_same_contents_changes_source_revision(self):
        before = self.revision()
        path = self.root / 'verification/aeneas/network/SandproofNetwork/Funs.lean'
        renamed = path.with_name('Renamed.lean')
        path.rename(renamed)
        renamed_revision = self.revision()
        self.assertNotEqual(before['sources_sha256'], renamed_revision['sources_sha256'])
        renamed.unlink()
        self.assertNotEqual(renamed_revision['sources_sha256'], self.revision()['sources_sha256'])

    def test_file_boundaries_cannot_be_merged_into_an_identical_hash_input(self):
        before = self.revision()
        first = self.root / 'verification/aeneas/network/SandproofNetwork/Funs.lean'
        second = self.root / 'verification/aeneas/network/SandproofNetwork/TypesExternal.lean'
        # These are consecutive dependency entries. Under unframed concatenation
        # deleting the second and appending its path/content to the first gives
        # precisely the same hash input while changing the formal source tree.
        first.write_bytes(first.read_bytes() +
                          str(second.relative_to(self.root)).encode() + second.read_bytes())
        second.unlink()
        after = self.revision()
        self.assertEqual(before['file_sha256'], after['file_sha256'])
        self.assertNotEqual(before['sources_sha256'], after['sources_sha256'])

    def test_file_creation_order_and_checkout_path_do_not_change_revision(self):
        before = self.revision()
        with tempfile.TemporaryDirectory() as directory:
            other_root = Path(directory).resolve()
            self.write_files(other_root, dict(reversed(list(self.files.items()))))
            with patch.object(self.client, 'ROOT', other_root):
                self.assertEqual(before, self.revision())

    def test_trial_file_change_is_seen_even_outside_formal_source_directories(self):
        before = self.revision('artifacts/trial.lean')
        (self.root / 'artifacts/trial.lean').write_text('#check Int\n')
        after = self.revision('artifacts/trial.lean')
        self.assertNotEqual(before, after)
        self.assertNotEqual(before['file_sha256'], after['file_sha256'])
        self.assertEqual(before['sources_sha256'], after['sources_sha256'])

    def test_absolute_path_inside_project_is_accepted(self):
        self.assertEqual(self.revision(), self.revision(str(self.root / 'SandboxModel/Selected.lean')))

    def test_parent_traversal_and_absolute_outside_paths_are_rejected(self):
        for path in ('../outside.lean', str(self.root.parent / 'outside.lean'),
                     str(self.root.with_name(self.root.name + '-other') / 'proof.lean')):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'inside this project'):
                self.revision(path)

    def test_symlink_to_outside_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            outside = Path(directory) / 'proof.lean'
            outside.write_text('#check Nat\n')
            (self.root / 'artifacts/escape.lean').symlink_to(outside)
            with self.assertRaisesRegex(ValueError, 'inside this project'):
                self.revision('artifacts/escape.lean')


if __name__ == '__main__':
    unittest.main()
