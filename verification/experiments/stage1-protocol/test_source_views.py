"""Ordinary admission tests; no compiler, translator, Lean or product process."""
from pathlib import Path
import importlib.util
import json
import os
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location('protocol_' + name, HERE / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


view, inventory = load('local_view'), load('source_inventory')
runtime = load('formal_runtime')
spec = importlib.util.spec_from_file_location('protocol_json_extract',
    HERE / 'envelope-bytes/extract-json.py')
json_extract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(json_extract)


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


class ExtractionSelection(unittest.TestCase):
    def test_only_lane_is_omitted_and_roles_precede_the_cargo_delimiter(self):
        script = (HERE.parents[2] / 'scripts/regenerate-foundations-lean.sh').read_text()
        original = script
        args, roots = json_extract.selection(script)
        delimiter = args.index('--')
        role_roots = [r for r in roots if r.startswith('crate::protocol_leaves::')]
        self.assertEqual(len(role_roots), 7)
        self.assertEqual(len(set(role_roots)), 7)
        self.assertNotIn('crate::lane_current', roots)
        for root in role_roots:
            self.assertLess(args.index(root), delimiter)
            self.assertLess(args.index(root), args.index('crate::json_ids::tenant_decode'))
        self.assertEqual(script, original)
        with self.assertRaisesRegex(ValueError, 'public extraction script'):
            json_extract.selection(script.replace('--start-from crate::lane_current',
                '--start-from crate::protocol_leaves::session_decode --start-from crate::lane_current'))


class StableTools(unittest.TestCase):
    def test_same_installation_is_exposed_read_only_at_a_stable_alias(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'
            tools = Path(temporary) / 'fixed-tools'
            packages = Path(temporary) / 'fixed-packages'
            root.mkdir()
            tools.mkdir()
            packages.mkdir()
            (root / '.tools').symlink_to(tools, target_is_directory=True)
            (root / '.lake').mkdir()
            (root / '.lake/packages').symlink_to(packages, target_is_directory=True)
            settings = dict(SANDPROOF_PROOF_TOOL_ROOT=str(tools),
                SANDPROOF_FORMAL_PACKAGES=str(packages))
            runtime_dirs = []
            for name in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
                directory = Path(temporary) / name.lower()
                directory.mkdir()
                runtime_dirs.append(directory)
                settings[name] = str(directory)
            with patch.dict(os.environ, **settings):
                self.assertEqual(runtime.tool_root(root), tools)
                self.assertEqual(runtime.package_root(root), packages)
                expected = [part for path in [tools, packages, *runtime_dirs]
                            for part in ('--ro-bind', str(path), str(path))]
                self.assertEqual(runtime.tool_bind(root), expected)
                other = Path(temporary) / 'other-packages'
                other.mkdir()
                with self.assertRaisesRegex(ValueError, 'requested packages differ'):
                    runtime.package_root(root, other)

    def test_another_installation_cannot_replace_the_original(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'
            root.mkdir()
            (root / '.tools').mkdir()
            other = Path(temporary) / 'other-tools'
            other.mkdir()
            with patch.dict(os.environ, SANDPROOF_PROOF_TOOL_ROOT=str(other)):
                with self.assertRaisesRegex(ValueError, 'original fixed installation'):
                    runtime.tool_root(root)

    def test_rebound_source_alias_is_rejected_before_it_can_form_a_loop(self):
        with patch.dict(os.environ, SANDPROOF_PROOF_TOOL_ROOT=str(runtime.VIRTUAL / '.tools')):
            with patch.object(runtime.Path, 'resolve', return_value=runtime.VIRTUAL / '.tools'):
                with self.assertRaisesRegex(ValueError, 'stable tool alias'):
                    runtime.tool_root(Path('/unused-source'))


if __name__ == '__main__':
    unittest.main()
