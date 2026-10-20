"""Reject drift in source bindings and assumptions, not a Tokio proof."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from tools.check_notify_contract import ROOT, check, digest


class NotifyAdmissionTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name) / 'source'
        self.cargo = Path(temp.name) / 'cargo'
        self.env = json.loads((ROOT/'verification/notify-environment.json').read_text())
        for name in self.env['inputs']:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/name, destination)
        library = self.cargo/'registry/src/unit-test/tokio-1.49.0/src/sync/notify.rs'
        library.parent.mkdir(parents=True)
        library.write_text('// synthetic admission fixture only\n')
        self.env['tokio']['reviewed_sources']['src/sync/notify.rs'] = digest(library)
        mocked = patch('tools.check_notify_contract.check_b1', return_value={'tokio': {'version': '1.49.0'}})
        mocked.start()
        self.addCleanup(mocked.stop)
        self.seal()

    def seal(self):
        (self.root/'verification/notify-environment.json').write_text(json.dumps(self.env))
        (self.root/'verification/notify-inputs.sha256').write_text(''.join(
            digest(self.root/name)+'  '+name+'\n' for name in self.env['inputs']))

    def test_registered_sites_admitted(self):
        self.assertEqual(len(check(self.root, self.cargo)['use_points']), 9)

    def test_changed_production_source_rejected(self):
        path = self.root/'sandbox-guest-runtime/src/pty.rs'
        path.write_text(path.read_text().replace('self.closed.notify_waiters();', 'self.closed.notify_one();'))
        with self.assertRaisesRegex(ValueError, 'Frozen Notify input changed'):
            check(self.root, self.cargo)

    def test_changed_original_signal_rejected_after_resealing(self):
        path = self.root/'sandbox-guest-runtime/src/pty.rs'
        path.write_text(path.read_text().replace('self.closed.notify_waiters();', 'other.notify_waiters();'))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Reviewed Notify call anchor changed'):
            check(self.root, self.cargo)

    def test_prepare_after_predicate_rejected_after_resealing(self):
        path = self.root/'sandbox-guest-runtime/src/pty.rs'
        old = 'closing.as_mut().enable();\n        let descriptor = self.descriptor()?;'
        new = 'let descriptor = self.descriptor()?;\n        closing.as_mut().enable();'
        path.write_text(path.read_text().replace(old, new, 1))
        self.seal()
        with self.assertRaisesRegex(ValueError, 'prepare/check or publish/broadcast order changed'):
            check(self.root, self.cargo)

    def test_missing_input_rejected(self):
        path = self.root/'verification/notify-inputs.sha256'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n')
        with self.assertRaisesRegex(ValueError, 'Missing or duplicate Notify input'):
            check(self.root, self.cargo)

    def test_missing_audit_rejected_after_resealing(self):
        path = self.root/'SandboxModel/Boundaries/Notify/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n')
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Missing, duplicate or extra Notify audit'):
            check(self.root, self.cargo)

    def test_library_drift_rejected(self):
        path = self.cargo/'registry/src/unit-test/tokio-1.49.0/src/sync/notify.rs'
        path.write_text('// changed library\n')
        with self.assertRaisesRegex(ValueError, 'Reviewed Notify library source changed'):
            check(self.root, self.cargo)

    def test_missing_business_provider_rejected(self):
        self.env['use_points'][0]['caller_obligations'] = ''
        self.seal()
        with self.assertRaisesRegex(ValueError, 'lacks source, condition or provider'):
            check(self.root, self.cargo)

    def test_condvar_cannot_be_claimed_by_notify(self):
        self.env['use_points'][-1]['status'] = 'conditional-mechanical'
        self.env['use_points'][-1]['public_results'] = [self.env['theorems'][0]]
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Condvar cannot be relabelled'):
            check(self.root, self.cargo)

    def test_close_decision_cannot_be_trusted(self):
        self.env['use_points'][-2]['status'] = 'conditional-mechanical'
        self.env['use_points'][-2]['public_results'] = [self.env['theorems'][0]]
        self.seal()
        with self.assertRaisesRegex(ValueError, 'Close authorization is not trusted'):
            check(self.root, self.cargo)
