import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools.check_tokio_rwlock_contract import ROOT, SCOPE, check_bindings


class TokioRwLockContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = json.loads((ROOT / 'verification/tokio-rwlock-environment.json').read_text())
        names = list(self.env['production_sources']) + [
            f'SandboxModel/Boundaries/TokioRwLock/{name}.lean'
            for name in ['Core', 'Mapping', 'Examples', 'Audit']]
        for name in names:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)

    def test_original_bindings_are_complete(self):
        self.assertEqual(check_bindings(self.root, self.env)['scope'], SCOPE)

    def test_final_arc_payload_frame_cannot_be_silently_added(self):
        self.env['scope']['payload_frame_requires_surviving_original_lock'] = False
        with self.assertRaisesRegex(ValueError, 'scope'):
            check_bindings(self.root, self.env)

    def test_condition_model_cannot_claim_async_rust_refinement(self):
        self.env['scope']['async_rust_refinement'] = True
        with self.assertRaisesRegex(ValueError, 'scope'):
            check_bindings(self.root, self.env)

    def test_missing_original_guard_field_is_rejected(self):
        self.env['use_points'] = [p for p in self.env['use_points'] if p['id'] != 'permit.guard-field']
        with self.assertRaisesRegex(ValueError, 'registration'):
            check_bindings(self.root, self.env)

    def test_authentication_cannot_be_moved_to_trusted_wiring(self):
        self.env = copy.deepcopy(self.env)
        point = next(p for p in self.env['use_points'] if p['id'] == 'authorize.credentials')
        point['status'] = 'conditional-mechanical'
        point['public_results'] = [self.env['theorems'][0]]
        with self.assertRaisesRegex(ValueError, 'business decision'):
            check_bindings(self.root, self.env)

    def test_missing_dependency_audit_is_rejected(self):
        path = self.root / 'SandboxModel/Boundaries/TokioRwLock/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        with self.assertRaisesRegex(ValueError, 'audit'):
            check_bindings(self.root, self.env)


class TokioRwLockBuildBindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / 'target'
        self.target.mkdir()
        self.binary = self.target / 'session'
        self.library = self.target / 'libguest.rlib'
        self.binary.write_bytes(b'fixture executable')
        self.library.write_bytes(b'fixture library')
        manifest = str(self.root / 'sandbox-guest-service/Cargo.toml')
        self.rows = [dict(reason='compiler-artifact', fresh=False, manifest_path=manifest,
            target=dict(name='session', kind=['test']), executable=str(self.binary), filenames=[str(self.binary)]),
            dict(reason='compiler-artifact', fresh=False, manifest_path=manifest,
            target=dict(name='sandproof_guest_service', kind=['lib']), executable=None, filenames=[str(self.library)])]
        self.log = self.root / 'build.log'

    def check(self):
        from tools.check_tokio_rwlock_components import fresh_build
        self.log.write_text(''.join(json.dumps(row) + '\n' for row in self.rows))
        return fresh_build(self.log, self.root, self.target)

    def test_fresh_library_and_binary_bound_to_same_source_are_accepted(self):
        self.assertEqual(self.check(), (self.binary, self.library))

    def test_new_binary_cannot_hide_reused_mutant_library(self):
        self.rows[1]['fresh'] = True
        with self.assertRaisesRegex(ValueError, 'Both business library'):
            self.check()

    def test_other_source_copy_library_is_rejected(self):
        self.rows[1]['manifest_path'] = str(self.root / 'other/sandbox-guest-service/Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'source copy'):
            self.check()


if __name__ == '__main__':
    unittest.main()
