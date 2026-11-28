"""Fail closed on changed contracts, missing counterexamples and ineffective mutations."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('guard_candidate', HERE / 'guard_candidate.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class GuardAdmission(unittest.TestCase):
    def fixture(self, root):
        shutil.copytree(guard.ROOT / guard.PROOFS, root / guard.PROOFS)
        shutil.copytree(HERE, root / guard.HERE)
        parent = json.loads((HERE / 'inventory.json').read_text())
        files = list(parent['base_sources']) + [
            str(guard.HERE.parent / 'formal.py'),
            str(guard.HERE.parent / 'resume-observation/formal.py'),
            str(guard.HERE.parent / 'observation-bindings/extract.py'),
        ]
        for name in files:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(guard.ROOT / name, target)

    def rejection(self, change):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            change(root)
            with self.assertRaises(ValueError):
                guard.inventory(root)

    def test_complete_inventory(self):
        data, parent = guard.inventory(guard.ROOT)
        self.assertEqual(set(data['mutations']), set(parent['mutations']))
        self.assertEqual(len(data['roots']), 12)

    def test_missing_formal_mutant(self):
        self.rejection(lambda root:
            (root / guard.CANDIDATE / 'NegativeCreateWrap.lean.in').unlink())

    def test_unregistered_proof(self):
        self.rejection(lambda root:
            (root / guard.CANDIDATE / 'Unexpected.lean.in').write_text(''))

    def test_missing_audit(self):
        def change(root):
            p = root / guard.CANDIDATE / 'CreateObservationBindings.lean.in'
            lines = p.read_text().splitlines(keepends=True)
            p.write_text(''.join(line for line in lines if not line.startswith('#audit_controller_proof')))
        self.rejection(change)

    def test_changed_full_contract(self):
        self.rejection(lambda root:
            (root / guard.PROOFS / 'CreateObservationModel.lean').write_text('-- different contract\n'))

    def test_changed_witnesses(self):
        self.rejection(lambda root:
            (root / guard.PROOFS / 'CreateObservationValues.lean').write_text('-- different witnesses\n'))

    def test_changed_conversion_relation(self):
        self.rejection(lambda root:
            (root / guard.HERE / 'conversion-candidate/CreateConversionRepresentation.lean.in').write_text('-- changed\n'))

    def test_unapproved_adoption(self):
        def change(root):
            p = root / guard.CANDIDATE / 'inventory.json'
            data = json.loads(p.read_text())
            data['v0_adopted'] = True
            p.write_text(json.dumps(data))
        self.rejection(change)

    def test_missing_std_root(self):
        def change(root):
            p = root / guard.HERE.parent / 'observation-bindings/extract.py'
            p.write_text(p.read_text().replace("'create_signed_deadline'", "'different_root'"))
        self.rejection(change)

    def test_all_mutations_change_only_registered_expression(self):
        parent = json.loads((HERE / 'inventory.json').read_text())
        with tempfile.TemporaryDirectory() as directory:
            for case, mutation in parent['mutations'].items():
                root = Path(directory) / case
                path = root / mutation['source']
                path.parent.mkdir(parents=True)
                before = (guard.ROOT / mutation['source']).read_text()
                path.write_text(before)
                row = guard.mutate(root, mutation)
                self.assertEqual(path.read_text(), before.replace(mutation['old'], mutation['new'], 1))
                self.assertNotEqual(row['before_sha256'], row['after_sha256'])

    def test_unchanged_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'actual.rs').write_text('unique')
            with self.assertRaises(ValueError):
                guard.mutate(root, dict(source='actual.rs', old='unique', new='unique'))

    def test_ambiguous_mutation_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'actual.rs').write_text('duplicate duplicate')
            with self.assertRaises(ValueError):
                guard.mutate(root, dict(source='actual.rs', old='duplicate', new='changed'))


if __name__ == '__main__':
    unittest.main()
