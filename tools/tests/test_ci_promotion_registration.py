from dataclasses import asdict
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_promotion_acceptance as gate


class TestRegistration(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.registry = self.directory('registry')
        self.checkout = self.directory('checkout')
        self.candidate = gate.Candidate(681, 'integration', *[c * 40 for c in 'abcd'])
        for installed in (
                patch.object(gate, 'MOUNT', self.root),
                patch.object(gate, 'promotion_registry', return_value=self.registry),
                patch.object(gate.Candidate, 'check_checkout')):
            installed.start()
            self.addCleanup(installed.stop)

    def directory(self, name):
        path = self.root/name
        path.mkdir(mode=0o700)
        return path

    def run_attempt(self, path, **options):
        return gate.run_registered(self.candidate, self.checkout, path, **options)

    def prepare_original(self, out, *, seal=False):
        native = out/'native'
        native.mkdir(mode=0o700)
        gate.write_once(out/'attempt.json', dict(schema=1, candidate=asdict(self.candidate),
                                               native_path=str(native)))
        if seal:
            gate.write_once(native/'complete.json', {'original': 'native'})
            formal = out/'tmp'/'sandproof-stage1-protocol.old'/'registered'
            formal.mkdir(mode=0o700, parents=True)
            gate.write_once(formal/'registered-result.json', dict(profile='full',
                shared_seal_sha256=gate.digest(native/'complete.json'), complete=False))

    def test_lock_excludes_distinct_output_while_running_and_registration_excludes_it_after_exit(self):
        original = self.directory('first')
        other = self.directory('other')

        def entered(candidate, checkout, output, registration_fd, **options):
            self.assertEqual(output, original)
            key = gate.receipts.digest(asdict(candidate))
            self.assertEqual(os.fstat(registration_fd).st_ino, (self.registry/key/'run.lock').stat().st_ino)
            with self.assertRaisesRegex(ValueError, 'already has a running'):
                self.run_attempt(other)

        with patch.object(gate, 'run_stage', side_effect=entered) as stage:
            self.run_attempt(original)
            with self.assertRaisesRegex(ValueError, 'already registered'):
                self.run_attempt(other)
        stage.assert_called_once()
        self.assertEqual(list(other.iterdir()), [])

    def test_resume_requires_latest_attempt_and_keeps_its_native_seal(self):
        original, resumed, stale = [self.directory(name) for name in ('original', 'resumed', 'stale')]
        with patch.object(gate, 'run_stage', side_effect=lambda *a, **k: self.prepare_original(original, seal=True)):
            self.run_attempt(original)
        with patch.object(gate, 'run_stage') as stage:
            self.run_attempt(resumed, resume_from=original)
            self.assertEqual(stage.call_args.kwargs['resume_from'], original)
            self.assertIsNone(stage.call_args.kwargs['native'])
            with self.assertRaisesRegex(ValueError, 'latest recorded output'):
                self.run_attempt(stale, resume_from=original)
        stage.assert_called_once()
        self.assertTrue((original/'native'/'complete.json').is_file())

    def test_completed_attempt_remains_available_for_publication(self):
        original, fresh = [self.directory(name) for name in ('complete', 'fresh')]
        with patch.object(gate, 'run_stage'):
            self.run_attempt(original)
        gate.write_once(original/'complete.json', {'success': True})
        with patch.object(gate, 'run_stage') as stage:
            with self.assertRaisesRegex(ValueError, 'original output for publication'):
                self.run_attempt(fresh, resume_from=original)
        stage.assert_not_called()
        self.assertTrue((original/'complete.json').is_file())

    def test_failed_retired_native_can_adopt_original_but_unknown_attempt_cannot(self):
        failed, retry, original = [self.directory(name) for name in ('failed', 'retry', 'already-native')]
        with patch.object(gate, 'run_stage', side_effect=lambda *a, **k: self.prepare_original(failed)):
            self.run_attempt(failed)
        with patch.object(gate, 'run_stage') as resumed:
            with self.assertRaises(FileNotFoundError):
                self.run_attempt(retry, resume_from=failed, native=original)
            resumed.assert_not_called()
            native = failed/'native'
            identity = native.stat()
            gate.write_once(native/'native-task.json', dict(kind=gate.NATIVE_TASK_OWNER,
                candidate=self.candidate.head, scope=gate.SCOPE, identity=[identity.st_dev, identity.st_ino]))
            gate.write_once(native/'native-task-exit.json', {'returncode': 1})
            gate.write_once(native/'native-task-retired.json', dict(candidate=self.candidate.head,
                scope=gate.SCOPE, status='failed', returncode=1, remaining_build_bytes=0))
            self.run_attempt(retry, resume_from=failed, native=original)
        self.assertIsNone(resumed.call_args.kwargs['resume_from'])
        self.assertEqual(resumed.call_args.kwargs['native'], original)
        self.assertTrue((failed/'attempt.json').is_file())

    def test_cannot_restart_if_original_output_was_replaced(self):
        original, fresh = [self.directory(name) for name in ('first', 'fresh')]
        with patch.object(gate, 'run_stage'):
            self.run_attempt(original)
        original.rename(self.root/'archived-first')
        original.mkdir(mode=0o700)
        with patch.object(gate, 'run_stage') as stage:
            with self.assertRaisesRegex(ValueError, 'output was replaced'):
                self.run_attempt(fresh, resume_from=original)
        stage.assert_not_called()


if __name__ == '__main__':
    unittest.main()
