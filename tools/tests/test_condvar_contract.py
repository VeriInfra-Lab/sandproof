"""Test historical Condvar admission rules, not current Reaper refinement."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools.check_condvar_contract import SOURCE_FILES, check, digest
from tools.tests.proof_input_fixture import reference_root

# The #682 fixture manifest binds the original bytes to commit
# 56b14d642c392316043dfbdbe4d26bacc663fb30.
# Keep source and reviewed anchors together; never reseal current Reaper as old evidence.
ROOT = reference_root()


class CondvarAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'source'
        self.sysroots = {}
        self.env = json.loads((ROOT / 'verification/condvar-environment.json').read_text())
        for name in self.env['inputs']:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        for role, library in self.env['standard_library'].items():
            self.sysroots[role] = Path(temporary.name) / role
            for name in library['sources']:
                path = self.sysroots[role] / 'lib/rustlib/src/rust/library' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('// synthetic admission fixture only: ' + role + '/' + name + '\n')
                library['sources'][name] = digest(path)
        self.seal()

    def seal(self):
        self.env['production_sources'] = {name: digest(self.root / name) for name in SOURCE_FILES}
        (self.root / 'verification/condvar-environment.json').write_text(json.dumps(self.env))
        (self.root / 'verification/condvar-inputs.sha256').write_text(''.join(
            digest(self.root / name) + '  ' + name + '\n' for name in self.env['inputs']))

    def test_finite_registry_admitted(self):
        env = check(self.root, self.sysroots)
        self.assertEqual((len(env['use_points']), len(env['theorems'])), (7, 46))

    def test_wrong_original_mutex_rejected_after_reseal(self):
        path = self.root / 'sandbox-runtime/src/child_process/reaper.rs'
        source = path.read_text()
        self.assertEqual(source.count('self.wake.wait(pending)'), 1)
        path.write_text(source.replace('self.wake.wait(pending)', 'other.wake.wait(pending)'))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'call anchor changed'):
            check(self.root, self.sysroots)

    def test_notify_before_enqueue_rejected_after_reseal(self):
        path = self.root / 'sandbox-runtime/src/child_process/reaper.rs'
        source = path.read_text()
        push = '        reaper.pending.lock().unwrap().push((child, self));\n'
        notify = '        reaper.wake.notify_one();\n'
        self.assertEqual(source.count(push + notify), 1)
        path.write_text(source.replace(push + notify, notify + push))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'call order changed'):
            check(self.root, self.sysroots)

    def test_business_predicate_cannot_be_trusted(self):
        site = next(site for site in self.env['use_points'] if site['id'] == 'run.business-predicate')
        site.update(status='conditional-mechanical', public_results=[self.env['theorems'][0]])
        self.seal()
        with self.assertRaisesRegex(ValueError, 'business decision cannot be trusted'):
            check(self.root, self.sysroots)

    def test_public_wait_rule_cannot_be_shifted_to_business(self):
        site = next(site for site in self.env['use_points'] if site['id'] == 'run.wait')
        site.update(status='business-obligation', public_results=[])
        self.seal()
        with self.assertRaisesRegex(ValueError, 'public rule cannot be moved'):
            check(self.root, self.sysroots)

    def test_original_object_required(self):
        self.env['use_points'][0]['original_object'] = ''
        self.seal()
        with self.assertRaisesRegex(ValueError, 'lacks original object'):
            check(self.root, self.sysroots)

    def test_full_rust_claim_rejected(self):
        self.env['proof_kind'] = 'full-reaper-rust-refinement'
        self.seal()
        with self.assertRaisesRegex(ValueError, 'cannot claim full Reaper'):
            check(self.root, self.sysroots)

    def test_missing_input_rejected(self):
        path = self.root / 'verification/condvar-inputs.sha256'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        with self.assertRaisesRegex(ValueError, 'Missing or duplicate Condvar input'):
            check(self.root, self.sysroots)

    def test_missing_audit_rejected_after_reseal(self):
        path = self.root / 'SandboxModel/Boundaries/Condvar/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1]) + '\n')
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Missing, duplicate or extra Condvar audit'):
            check(self.root, self.sysroots)

    def test_fixed_futex_library_drift_rejected(self):
        path = self.sysroots['frontend'] / 'lib/rustlib/src/rust/library/std/src/sys/sync/condvar/futex.rs'
        path.write_text('// different original unlock/relock\n')
        with self.assertRaisesRegex(ValueError, 'Reviewed Condvar library source changed'):
            check(self.root, self.sysroots)
