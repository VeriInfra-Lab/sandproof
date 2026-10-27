"""Ordinary admission tests; no compiler, translator, Lean or product process."""
from pathlib import Path
import importlib.util
import json
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location('protocol_' + name, HERE / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


view, inventory = load('local_view'), load('source_inventory')


class PrivateView(unittest.TestCase):
    def prepare(self, root):
        files = {
            'lakefile.toml': '# original public view\n',
            str(view.EXTRACT_ROOT): '// original transparent extraction roots\n',
            str(view.ENCODING): 'inductive JsonOutput where\n  | unsigned (value : Nat)\n',
            str(view.HERE / 'local-view.toml.in'): (HERE / 'local-view.toml.in').read_text(),
            str(view.HERE / 'schema-leaves/protocol_leaves.rs.in'): '// transparent root\n',
            str(view.HERE / 'schema-leaves/RoleLeaves.lean.in'): '-- role proof\n',
            str(view.HERE / 'envelope-schema/EnvelopeSchema.lean.in'): '-- schema proof\n',
            'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean':
                '-- original public provider\n',
            'scripts/regenerate-foundations-lean.sh': '# original public generator\n',
        }
        for name, text in files.items():
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)
        return files

    def test_private_view_preserves_public_provider_generator_and_lints(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            before = self.prepare(root)
            changed = view.install(root, 'envelope-schema')
            for name in before:
                if name not in changed:
                    self.assertEqual((root / name).read_text(), before[name])
            self.assertEqual((root / 'lakefile.toml').read_bytes(),
                             (HERE / 'local-view.toml.in').read_bytes())
            self.assertNotIn('linter', (root / 'lakefile.toml').read_text())
            self.assertNotIn('tools/proof_batch.py', changed)
            self.assertIn('  | signed (value : Int)', (root / view.ENCODING).read_text())
            self.assertIn('  | boolean (value : Bool)', (root / view.ENCODING).read_text())

    def test_install_cannot_silently_apply_twice(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare(root)
            view.install(root, 'schema-leaves')
            with self.assertRaisesRegex(ValueError, 'already installed'):
                view.install(root, 'schema-leaves')

    def test_changed_output_shape_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.prepare(root)
            (root / view.ENCODING).write_text('unreviewed output syntax\n')
            with self.assertRaisesRegex(ValueError, 'output syntax'):
                view.install(root, 'envelope-schema')

    def test_original_provider_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = Path(temporary) / 'FunsExternal.lean'
            original.write_text('import Aeneas\n-- original provider\n')
            before = original.read_bytes()
            adapted = view.role_provider(original)
            self.assertEqual(original.read_bytes(), before)
            self.assertIn('abbrev core.result.Result.map := @SandproofProtocolResultRust', adapted)
            original.write_text(adapted)
            with self.assertRaisesRegex(ValueError, 'unexpectedly includes'):
                view.role_provider(original)


class RetainedInventory(unittest.TestCase):
    def test_missing_extra_duplicate_and_unregistered_required_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / 'SandboxModel/Lifecycle/Protocol'
            directory.mkdir(parents=True)
            (directory / 'One.lean').write_text('-- one\n')
            manifest = root / inventory.HERE / 'retained-protocol.json'
            manifest.parent.mkdir(parents=True)
            def save(names):
                manifest.write_text(json.dumps(dict(schema=1, modules=names)))
            save(['One'])
            self.assertEqual(inventory.retained(root, ['One']), {'One'})
            for names, required in [([], []), (['One', 'Missing'], []),
                                    (['One', 'One'], []), (['One'], ['Missing'])]:
                save(names)
                with self.assertRaisesRegex(ValueError, 'unregistered or missing'):
                    inventory.retained(root, required)
            save(['One'])
            (directory / 'Extra.lean').write_text('-- extra\n')
            with self.assertRaisesRegex(ValueError, 'unregistered or missing'):
                inventory.retained(root, ['One'])


if __name__ == '__main__':
    unittest.main()
