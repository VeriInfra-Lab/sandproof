"""Admission regressions for finite JoinSet bindings, not tests of Tokio correctness."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from tools.check_joinset_contract import ROOT, check, digest, SOURCE_FILES


class JoinSetAdmissionTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)/'source'
        self.cargo = Path(temp.name)/'cargo'
        self.env = json.loads((ROOT/'verification/joinset-environment.json').read_text())
        for name in self.env['inputs']:
            destination = self.root/name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/name, destination)
        for name in self.env['tokio']['reviewed_sources']:
            library = self.cargo/'registry/src/unit-test/tokio-1.49.0'/name
            library.parent.mkdir(parents=True, exist_ok=True)
            library.write_text('// synthetic admission fixture only: '+name+'\n')
            self.env['tokio']['reviewed_sources'][name] = digest(library)
        mocked = patch('tools.check_joinset_contract.check_b1',
                       return_value={'tokio': {'version': '1.49.0'}})
        mocked.start()
        self.addCleanup(mocked.stop)
        self.seal()

    def seal(self):
        self.env['production_sources'] = {n: digest(self.root/n) for n in SOURCE_FILES}
        (self.root/'verification/joinset-environment.json').write_text(json.dumps(self.env))
        (self.root/'verification/joinset-inputs.sha256').write_text(''.join(
            digest(self.root/name)+'  '+name+'\n' for name in self.env['inputs']))

    def test_registered_original_groups_admitted(self):
        self.assertEqual(len(check(self.root, self.cargo)['use_points']), 15)

    def test_changed_source_rejected(self):
        path = self.root/'sandbox-internal/src/protected_server.rs'
        path.write_text(path.read_text().replace('tasks.abort_all();', 'other.abort_all();'))
        with self.assertRaisesRegex(ValueError, 'Frozen JoinSet input changed'):
            check(self.root, self.cargo)

    def test_changed_original_group_rejected_after_resealing(self):
        path = self.root/'sandbox-internal/src/protected_server.rs'
        path.write_text(path.read_text().replace('tasks.abort_all();', 'other.abort_all();'))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Reviewed JoinSet call anchor changed'):
            check(self.root, self.cargo)

    def test_abort_after_join_rejected_after_resealing(self):
        path = self.root/'sandbox-internal/src/protected_server.rs'
        path.write_text(path.read_text().replace(
            'tasks.abort_all();\n        while tasks.join_next().await.is_some() {}',
            'while tasks.join_next().await.is_some() {}\n        tasks.abort_all();'))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'abort/drain order changed'):
            check(self.root, self.cargo)

    def test_new_production_group_cannot_disappear_from_census(self):
        path = self.root/'sandbox-new/src/lib.rs'
        path.parent.mkdir(parents=True)
        path.write_text('use tokio::task::JoinSet;\n')
        with self.assertRaisesRegex(ValueError, 'production-file census changed'):
            check(self.root, self.cargo)

    def test_missing_input_rejected(self):
        path = self.root/'verification/joinset-inputs.sha256'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n')
        with self.assertRaisesRegex(ValueError, 'Missing or duplicate JoinSet input'):
            check(self.root, self.cargo)

    def test_missing_audit_rejected_after_resealing(self):
        path = self.root/'SandboxModel/Boundaries/JoinSet/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n')
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Missing, duplicate or extra JoinSet audit'):
            check(self.root, self.cargo)

    def test_library_harness_drift_rejected(self):
        path = self.cargo/'registry/src/unit-test/tokio-1.49.0/src/runtime/task/harness.rs'
        path.write_text('// changed library\n')
        with self.assertRaisesRegex(ValueError, 'Reviewed JoinSet library source changed'):
            check(self.root, self.cargo)

    def test_missing_business_provider_rejected(self):
        self.env['use_points'][0]['caller_obligations'] = ''
        self.seal()
        with self.assertRaisesRegex(ValueError, 'lacks source, condition or provider'):
            check(self.root, self.cargo)

    def test_business_callback_cannot_be_trusted(self):
        row = next(r for r in self.env['use_points'] if r['id'] == 'protected.callback-policy')
        row['status'] = 'conditional-mechanical'
        row['public_results'] = [self.env['theorems'][0]]
        self.seal()
        with self.assertRaisesRegex(ValueError, 'business callback or result decision is not trusted'):
            check(self.root, self.cargo)

    def test_public_rule_cannot_be_moved_to_business(self):
        row = next(r for r in self.env['use_points'] if r['id'] == 'guest.normal-drain')
        row['status'] = 'business-obligation'
        row['public_results'] = []
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Required public JoinSet rule cannot be moved'):
            check(self.root, self.cargo)

    def test_duplicate_use_point_rejected(self):
        self.env['use_points'].append(dict(self.env['use_points'][0]))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Finite JoinSet use-point registration changed'):
            check(self.root, self.cargo)
