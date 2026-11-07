"""Admission regressions: stale sources and incomplete proof inventories must fail."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('wire_formal', HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.directory = self.root / formal.TEMPLATES
        shutil.copytree(HERE / 'accepted', self.directory)
        self.data = json.loads((self.directory / 'inventory.json').read_text())
        for name in self.data['base_sources']:
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(formal.ROOT / name, p)

    def save(self):
        (self.directory / 'inventory.json').write_text(json.dumps(self.data))

    def test_complete_adopted_inventory_is_admitted(self):
        self.assertTrue(formal.inventory(self.root)['production_adopted'])

    def test_changed_production_source_is_rejected(self):
        (self.root / 'sandbox-internal/src/lifecycle.rs').write_text('changed source\n')
        with self.assertRaisesRegex(ValueError, 'fixed production source changed'):
            formal.inventory(self.root)

    def test_unregistered_template_is_rejected(self):
        (self.directory / 'Unregistered.lean.in').write_text('example : True := by trivial\n')
        with self.assertRaisesRegex(ValueError, 'unregistered proof template'):
            formal.inventory(self.root)

    def test_missing_audit_is_rejected(self):
        p = self.directory / 'AcceptedWire.lean.in'
        p.write_text(p.read_text().replace('#audit_controller_proof', '-- removed audit', 1))
        with self.assertRaisesRegex(ValueError, 'audit declaration differs'):
            formal.inventory(self.root)

    def test_duplicate_audit_even_in_inventory_is_rejected(self):
        name = 'AcceptedWire'
        audit = self.data['audits'][name][0]
        p = self.directory / (name + '.lean.in')
        p.write_text(p.read_text() + '\n#audit_controller_proof ' + audit + '\n')
        self.data['audits'][name].append(audit)
        self.save()
        with self.assertRaisesRegex(ValueError, 'duplicate proof audit'):
            formal.inventory(self.root)

    def test_unadopted_candidate_is_rejected(self):
        self.data['production_adopted'] = False
        self.save()
        with self.assertRaisesRegex(ValueError, 'requires the adopted production entry'):
            formal.inventory(self.root)


if __name__ == '__main__':
    unittest.main()
