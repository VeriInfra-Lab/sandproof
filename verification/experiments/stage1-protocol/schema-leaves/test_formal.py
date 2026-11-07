"""Admission failures must remain failures before any expensive extraction."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('role_formal', HERE / 'formal.py')
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
        self.assertEqual(len(formal.inventory(self.root)['audits']), 23)

    def test_changed_production_is_rejected(self):
        path = self.root / 'sandbox-protocol/src/lifecycle/identity.rs'
        path.write_text(path.read_text() + '\n// changed\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_changed_shared_diff_is_rejected(self):
        path = self.root / formal.HERE / 'shared-candidate.patch'
        path.write_text(path.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_missing_audit_is_rejected(self):
        self.data['audits'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'audit'):
            formal.inventory(self.root)

    def test_duplicate_audit_is_rejected(self):
        self.data['audits'].append(self.data['audits'][0])
        self.save()
        with self.assertRaisesRegex(ValueError, 'audit'):
            formal.inventory(self.root)

    def test_missing_role_is_rejected(self):
        self.data['roles'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'role'):
            formal.inventory(self.root)

    def test_duplicate_role_is_rejected(self):
        self.data['roles'].append('session')
        self.save()
        with self.assertRaisesRegex(ValueError, 'role'):
            formal.inventory(self.root)

    def test_candidate_cannot_claim_public_adoption(self):
        self.data['v0_adopted'] = True
        self.save()
        with self.assertRaisesRegex(ValueError, 'unadopted'):
            formal.inventory(self.root)

    def test_prepare_race_cannot_leave_success_flag(self):
        inputs = dict(self.data['inputs'])
        # The second snapshot detects a source change after successful patching.
        with patch.object(formal, 'ROOT', self.root), patch.object(
            formal, 'snapshot', side_effect=[inputs, dict(inputs, changed='changed')]
        ), patch.object(formal.subprocess, 'check_output', return_value='reviewed-base\n'):
            result = formal.run(self.root / 'output', prepare_only=True)
        self.assertFalse(result['complete'])
        self.assertFalse(result.get('prepared', False))
        self.assertIn('changed during', result['error'])


if __name__ == '__main__':
    unittest.main()
