"""Admission regressions for missing proofs and stale/foreign compiled artifacts."""
from pathlib import Path
import copy
import importlib.util
import json
import tempfile
import shutil
import unittest

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location('renewal_' + name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


formal, native = load('formal'), load('native')


class Admissions(unittest.TestCase):
    def test_complete_source_inventory(self):
        data = formal.inventory(formal.ROOT)
        self.assertEqual(set(data['mutations']), set(native.MUTANT_TESTS))
        self.assertEqual(len(data['audits']['RenewalBindings']), 9)

    def test_missing_proof_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(formal.ROOT / formal.PROOFS, root / formal.PROOFS)
            (root / formal.HERE).mkdir(parents=True)
            shutil.copy2(formal.ROOT / formal.HERE / 'inventory.json', root / formal.HERE / 'inventory.json')
            shutil.copy2(formal.ROOT / formal.HERE.parent / 'formal.py', root / formal.HERE.parent / 'formal.py')
            data = json.loads((root / formal.HERE / 'inventory.json').read_text())
            for name in data['base_sources']:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(formal.ROOT / name, target)
            formal.inventory(root)  # The unchanged fixture must be admitted first.
            (root / formal.PROOFS / 'RenewalBindings.lean').unlink()
            with self.assertRaisesRegex(ValueError, 'unregistered or missing retained protocol proof'):
                formal.inventory(root)

    def test_changed_result_tag_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            llbc = root / 'changed.llbc'
            declaration = dict(item_meta=dict(name=[{'Ident': [x, 0]} for x in ['core', 'result', 'Result']]),
                kind=dict(Enum=[dict(name='Ok', discriminant={'Signed': ['Isize', '1']}, fields=[{}]),
                                dict(name='Err', discriminant={'Signed': ['Isize', '0']}, fields=[{}])]))
            llbc.write_text(json.dumps(dict(translated=dict(type_decls=[declaration]))))
            with self.assertRaisesRegex(ValueError, 'original Result tags differ'):
                formal.install_result_tags(root, llbc)

    def test_missing_result_representation_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            llbc = root / 'missing.llbc'
            llbc.write_text(json.dumps(dict(translated=dict(type_decls=[]))))
            with self.assertRaisesRegex(ValueError, 'missing or ambiguous original Result'):
                formal.install_result_tags(root, llbc)

    def compiler_fixture(self, root):
        source, target = root / 'source', root / 'target'
        source.mkdir()
        target.mkdir()
        rows = []
        for name, kind, manifest, relative in [
            ('sandproof_protocol', 'lib', Path('sandbox-protocol/Cargo.toml'), Path('sandbox-protocol/src/lib.rs')),
            ('sandproof_stage1_result_confirmation_extract', 'lib', native.CRATE / 'Cargo.toml', native.CRATE / 'src/lib.rs'),
            ('renewal', 'test', native.CRATE / 'Cargo.toml', native.CRATE / 'tests/renewal.rs'),
        ]:
            output = target / name
            output.write_bytes(b'compiled fixture')
            rows.append(dict(reason='compiler-artifact', fresh=False,
                manifest_path=str(source / manifest),
                target=dict(name=name, kind=[kind], src_path=str(source / relative)),
                profile=dict(test=kind == 'test'), executable=str(output) if kind == 'test' else None,
                filenames=[str(output)]))
        return source, target, rows

    def check_rows(self, mutate=None, reject=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target, rows = self.compiler_fixture(root)
            if mutate:
                mutate(rows, root)
            log = root / 'compiler.log'
            log.write_text(''.join(json.dumps(row)+'\n' for row in rows))
            if reject:
                with self.assertRaises(ValueError):
                    native.compiler_artifacts(source, target, log)
            else:
                self.assertEqual(len(native.compiler_artifacts(source, target, log)), 3)

    def test_fresh_complete_program_admitted(self):
        self.check_rows()

    def test_stale_dependency_rejected(self):
        self.check_rows(lambda rows, _: rows[0].update(fresh=True), True)

    def test_foreign_manifest_rejected(self):
        self.check_rows(lambda rows, root: rows[0].update(manifest_path=str(root / 'foreign/Cargo.toml')), True)

    def test_missing_dependency_rejected(self):
        self.check_rows(lambda rows, _: rows.pop(0), True)

    def test_duplicate_test_rejected(self):
        self.check_rows(lambda rows, _: rows.append(copy.deepcopy(rows[-1])), True)

    def test_escaped_binary_rejected(self):
        def mutate(rows, root):
            p = root / 'foreign-binary'
            p.write_bytes(b'foreign')
            rows[-1]['filenames'] = [str(p)]
        self.check_rows(mutate, True)


if __name__ == '__main__':
    unittest.main()
