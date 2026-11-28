"""Registration must reject omissions before copying sources or invoking Lean."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('manifest_validation_formal',
    Path(__file__).with_name('manifest_validation_formal.py'))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class RegistrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for directory, names in ((driver.HERE, ['inventory.json', 'NegativeFullCapture.lean.in']),
                                 (driver.BASE, ['inventory.json', 'mutations.json']),
                                 (driver.PROTOCOL, ['negative_native.py'])):
            (self.root / directory).mkdir(parents=True, exist_ok=True)
            for name in names:
                shutil.copy2(driver.ROOT / directory / name, self.root / directory / name)
        (self.root / driver.PROOFS).mkdir(parents=True)
        for path in (driver.ROOT / driver.PROOFS).glob('Manifest*.lean'):
            shutil.copy2(path, self.root / driver.PROOFS / path.name)

    def change_inventory(self, change):
        path = self.root / driver.HERE / 'inventory.json'
        value = json.loads(path.read_text())
        change(value)
        path.write_text(json.dumps(value))

    def test_current_inventory_covers_complete_slice(self):
        inventory, _, mutations = driver.read_inventory(self.root)
        self.assertEqual(set(inventory['cases']), {'correct', *mutations})

    def test_new_file_cannot_be_silently_ignored(self):
        (self.root / driver.PROOFS / 'ManifestForgotten.lean').write_text('-- new obligation\n')
        with self.assertRaisesRegex(ValueError, 'unregistered'):
            driver.read_inventory(self.root)

    def test_correct_case_cannot_omit_registered_module(self):
        self.change_inventory(lambda value: value['cases']['correct']['proofs'].remove('ManifestProfile'))
        with self.assertRaisesRegex(ValueError, 'every manifest validation module'):
            driver.read_inventory(self.root)

    def test_actual_rust_mutation_cannot_lose_counterexample(self):
        self.change_inventory(lambda value: value['cases'].pop('skip-file-path'))
        with self.assertRaisesRegex(ValueError, 'every registered mutation'):
            driver.read_inventory(self.root)

    def test_capture_mutation_cannot_lose_counterexample(self):
        self.change_inventory(lambda value: value['cases'].pop(driver.CAPTURE_CASE))
        with self.assertRaisesRegex(ValueError, 'every registered mutation'):
            driver.read_inventory(self.root)

    def test_historical_normal_witness_must_be_checked(self):
        self.change_inventory(lambda value: value['cases']['correct']['proofs'].remove('Manifest'))
        with self.assertRaisesRegex(ValueError, 'every manifest validation module'):
            driver.read_inventory(self.root)

    def test_capture_supplement_cannot_be_omitted(self):
        (self.root / driver.HERE / 'NegativeFullCapture.lean.in').unlink()
        with self.assertRaisesRegex(ValueError, 'capture counterexample supplement'):
            driver.read_inventory(self.root)

    def test_capture_helper_adaptation_requires_actual_declarations(self):
        generated = self.root / driver.LIBRARY / 'SandproofStage1Manifest/Funs.lean'
        generated.parent.mkdir(parents=True)
        generated.write_text('-- Missing actual loop0 output\n')
        witness = self.root / driver.PROOFS / 'ManifestWitnesses.lean'
        before = witness.read_bytes()
        with self.assertRaisesRegex(ValueError, 'actual capture mutant loop declarations'):
            driver.adapt_capture_helpers(self.root)
        self.assertEqual(witness.read_bytes(), before)

    def test_duplicate_audit_is_rejected(self):
        self.change_inventory(lambda value: value['cases']['correct']['audits'].append(
            value['cases']['correct']['audits'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate audit'):
            driver.read_inventory(self.root)


if __name__ == '__main__':
    unittest.main()
