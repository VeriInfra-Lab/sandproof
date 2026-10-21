"""Ordinary-machine rejection tests for component evidence admission."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from check import ROOT, HERE, MODEL, LIBRARY, check_inputs, sha


class FrozenInputsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='protected-directory-check-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.frozen_path = self.root / HERE.relative_to(ROOT) / 'frozen.json'
        frozen = json.loads((HERE / 'frozen.json').read_text())
        environment = json.loads((HERE / 'environment.json').read_text())
        for name in set(frozen['files']) | set(environment['base_inputs']) | {str(HERE.relative_to(ROOT) / 'frozen.json')}:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def test_exact_component_is_admitted(self):
        self.assertTrue(check_inputs(self.root)['files'])

    def test_changed_real_business_source_is_rejected(self):
        path = self.root / 'sandbox-internal/src/protected_files.rs'
        path.write_text(path.read_text().replace('0o022', '0o000'))
        with self.assertRaisesRegex(ValueError, 'frozen input differs'):
            check_inputs(self.root)

    def test_changed_generated_function_is_rejected(self):
        path = self.root / LIBRARY / 'correct/SandproofProtectedDirectory/Funs.lean'
        path.write_text(path.read_text() + '\n-- changed\n')
        with self.assertRaisesRegex(ValueError, 'frozen input differs'):
            check_inputs(self.root)

    def test_unregistered_proof_is_rejected(self):
        (self.root / MODEL / 'Extra.lean').write_text('import Aeneas\n')
        with self.assertRaisesRegex(ValueError, 'unregistered'):
            check_inputs(self.root)

    def test_removed_audit_is_rejected_even_if_hash_is_updated(self):
        path = self.root / MODEL / 'BadAudit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        frozen = json.loads(self.frozen_path.read_text())
        frozen['files'][str(path.relative_to(self.root))] = sha(path)
        self.frozen_path.write_text(json.dumps(frozen))
        with self.assertRaisesRegex(ValueError, 'sixteen-theorem'):
            check_inputs(self.root)

    def test_missing_actual_proof_is_rejected(self):
        (self.root / MODEL / 'Refinement.lean').unlink()
        with self.assertRaises(FileNotFoundError):
            check_inputs(self.root)


if __name__ == '__main__':
    unittest.main()
