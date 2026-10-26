"""Reject incomplete representation inventories before invoking proof tools."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('envelope_formal', HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = json.loads((HERE / 'inventory.json').read_text())
        for name in self.data['inputs']:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(formal.ROOT / name, target)
        self.save()

    def save(self):
        (self.root / formal.HERE / 'inventory.json').write_text(json.dumps(self.data))

    def test_current_reviewed_candidate_is_admitted(self):
        self.assertEqual(len(formal.inventory(self.root)['audits']), 24)

    def test_changed_source_is_rejected(self):
        source = self.root / 'sandbox-protocol/src/lifecycle/request.rs'
        source.write_text(source.read_text() + '\n// changed\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_missing_struct_is_rejected(self):
        del self.data['structs']['RecordedLifecycleIntent']
        self.save()
        with self.assertRaisesRegex(ValueError, 'structure'):
            formal.inventory(self.root)

    def test_missing_branch_is_rejected(self):
        del self.data['operations']['Destroy']
        self.save()
        with self.assertRaisesRegex(ValueError, 'branch'):
            formal.inventory(self.root)

    def test_field_reordering_is_rejected(self):
        self.data['structs']['OperationKey']['fields_in_declaration_order'].reverse()
        self.save()
        with self.assertRaisesRegex(ValueError, 'coverage'):
            formal.inventory(self.root)

    def test_signed_field_cannot_be_unsigned(self):
        self.data['operations']['Renew'][1][1] = 'u64'
        self.save()
        with self.assertRaisesRegex(ValueError, 'operation fields'):
            formal.inventory(self.root)

    def test_missing_audit_is_rejected(self):
        self.data['audits'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'audit'):
            formal.inventory(self.root)

    def test_adoption_cannot_be_inferred(self):
        self.data['a6_instance_adopted'] = True
        self.save()
        with self.assertRaisesRegex(ValueError, 'unadopted'):
            formal.inventory(self.root)

    def test_prepare_race_cannot_leave_success_flag(self):
        inputs = dict(self.data['inputs'])
        with patch.object(formal, 'ROOT', self.root), patch.object(
            formal, 'snapshot', side_effect=[inputs, dict(inputs, changed='changed')]
        ), patch.object(formal.subprocess, 'check_output', return_value='reviewed-base\n'):
            result = formal.run(self.root / 'output', prepare_only=True)
        self.assertFalse(result['complete'])
        self.assertFalse(result.get('prepared', False))
        self.assertIn('changed during', result['error'])


if __name__ == '__main__':
    unittest.main()
