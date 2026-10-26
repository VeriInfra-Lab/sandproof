"""The new proof admission path must reject incomplete or changed evidence."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('bytes_formal', HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.data = json.loads((HERE / 'inventory.json').read_text())
        for name in self.data['inputs']:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(formal.ROOT / name, target)
        self.save()

    def save(self):
        (self.root / formal.HERE / 'inventory.json').write_text(json.dumps(self.data))

    def test_current_candidate(self):
        self.assertEqual(len(formal.inventory(self.root)['audits']), 23)

    def test_production_change(self):
        path = self.root / 'sandbox-protocol/src/lifecycle/authority.rs'
        path.write_text(path.read_text() + '\n// changed\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_missing_audit(self):
        self.data['audits'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'audit'):
            formal.inventory(self.root)

    def test_missing_byte_root(self):
        self.data['byte_roots'].pop('recorded_decode')
        self.save()
        with self.assertRaisesRegex(ValueError, 'root'):
            formal.inventory(self.root)

    def test_extra_omitted_root(self):
        self.data['omitted_roots'].append('crate::counter_from_u64')
        self.save()
        with self.assertRaisesRegex(ValueError, 'selection'):
            formal.inventory(self.root)

    def test_generated_body_change(self):
        path = self.root / 'verification/aeneas/stage1-protocol/json-foundations/SandproofFoundations/Funs.lean'
        path.write_text(path.read_text() + '\n-- changed\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_cannot_claim_adopted_parser(self):
        self.data['a6_instance_adopted'] = True
        self.save()
        with self.assertRaisesRegex(ValueError, 'unadopted'):
            formal.inventory(self.root)

    def test_source_race_revokes_preparation(self):
        inputs = dict(self.data['inputs'])
        with patch.object(formal, 'ROOT', self.root), patch.object(formal, 'snapshot',
                side_effect=[inputs, dict(inputs, changed='changed')]), patch.object(
                formal.subprocess, 'check_output', return_value='reviewed-base\n'):
            result = formal.run(self.root / 'output', prepare_only=True)
        self.assertFalse(result['complete'])
        self.assertFalse(result.get('prepared', False))
        self.assertIn('changed during', result['error'])


if __name__ == '__main__':
    unittest.main()
