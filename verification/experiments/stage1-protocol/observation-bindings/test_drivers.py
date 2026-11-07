"""Admission regressions for missing proofs and stale/foreign compiled artifacts."""
from pathlib import Path
import copy
import importlib.util
import json
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location('result_' + name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


formal, native = load('formal'), load('native')


class Admissions(unittest.TestCase):
    def test_complete_source_inventory(self):
        data = formal.inventory(formal.ROOT)
        self.assertEqual(set(data['mutations']), set(native.MUTANT_TESTS))
        self.assertEqual(len(data['audits']['ReadyFactBindings']), 15)

    def compiler_fixture(self, root):
        source, target = root / 'source', root / 'target'
        source.mkdir()
        target.mkdir()
        rows = []
        for name, kind, manifest, relative in [
            ('sandproof_protocol', 'lib', Path('sandbox-protocol/Cargo.toml'), Path('sandbox-protocol/src/lib.rs')),
            ('sandproof_stage1_observation_bindings_extract', 'lib', native.HERE / 'Cargo.toml', native.HERE / 'src/lib.rs'),
            ('contract', 'test', native.HERE / 'Cargo.toml', native.HERE / 'tests/contract.rs'),
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
