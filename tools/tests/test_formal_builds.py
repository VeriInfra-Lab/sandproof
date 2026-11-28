import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

from tools import formal_builds, native_task
from tools.system_acceptance import digest, json_file


class FormalBuildTests(unittest.TestCase):
    candidate = 'a' * 40
    targets = ('correct/source/artifacts/extract-target', 'counterexample/source/artifacts/extract-target')

    def setUp(self):
        previous = os.umask(0o077)
        self.addCleanup(os.umask, previous)
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.parent = Path(directory.name).resolve()
        self.run = self.parent/'run'
        self.run.mkdir(mode=0o700)
        self.state = self.run/formal_builds.DIRECTORY
        self.observer = self.parent/'installed-observer'
        self.observer.write_text('tool observer call intercepted\n')
        self.receipt = self.run/'proof-receipt.json'
        self.source = self.run/'original.llbc'
        self.source.write_bytes(b'raw proof input')

    def enroll(self, *, targets=None):
        formal_builds.enroll(self.run, candidate=self.candidate, observer=self.observer,
                             targets=self.targets if targets is None else targets)

    def launch(self, suffix=''):
        code = f'''from pathlib import Path
import os, signal, sys
assert os.environ['FORMAL_BUILD_TEST'] == 'isolated environment'
for relative in {self.targets!r}:
    target = Path(relative)
    target.mkdir(parents=True, exist_ok=True)
    (target/'build-output').write_bytes(b'x'*8192)
Path('proof-receipt.json').write_text('original formal result')
print('logged by original proof subprocess')
{suffix}
'''
        return formal_builds.execute(self.run, [sys.executable, '-B', '-c', code],
                                     cwd=self.run, env={'FORMAL_BUILD_TEST': 'isolated environment'})

    def readback(self, _terminal):
        return {self.receipt: digest(self.receipt), self.source: digest(self.source)}

    def retire(self, readback=None, **kwargs):
        with patch.object(native_task, 'observe', **kwargs):
            return formal_builds.retire(self.run, self.observer, readback or self.readback)

    def assert_targets(self, present):
        self.assertTrue(all((self.run/path).exists() == present for path in self.targets))

    def test_original_success_and_evidence_remain_after_retirement(self):
        self.enroll()
        (self.run/self.targets[0]).mkdir(parents=True)  # Retained prelude can allocate empty Cargo storage.
        lean = self.run/'correct/source/.lake/build/original.olean'
        lean.parent.mkdir(parents=True)
        lean.write_bytes(b'persist Lean cache')
        self.assertEqual(self.launch()['returncode'], 0)
        before = [digest(path) for path in (self.source, self.receipt, lean)]
        result = self.retire()
        self.assert_targets(False)
        self.assertEqual(result['status'], 'passed')
        self.assertEqual(result['target_count'], 2)
        self.assertGreaterEqual(result['allocated_bytes'], 2*8192)
        self.assertEqual([digest(path) for path in (self.source, self.receipt, lean)], before)
        self.assertIn('logged by original proof', (self.state/'native-task.log').read_text())
        self.assertEqual(self.retire(), result)

    def test_failed_and_cancelled_retain_real_nonzero_outcome(self):
        for original, code, name, suffix in (
                ('failure', 19, 'failed', 'sys.exit(19)'),
                ('cancel', -signal.SIGTERM, 'cancelled', 'os.kill(os.getpid(), signal.SIGTERM)')):
            with self.subTest(name=name):
                self.run = self.parent/original
                self.run.mkdir()
                self.state = self.run/formal_builds.DIRECTORY
                self.receipt = self.run/'proof-receipt.json'
                self.enroll()
                self.assertEqual(self.launch(suffix)['returncode'], code)
                result = self.retire()
                self.assertEqual((result['status'], result['returncode']), (name, code))
                self.assert_targets(False)

    def test_unknown_or_live_reference_retains_original_targets(self):
        self.enroll()
        self.launch('os.kill(os.getpid(), signal.SIGKILL)')
        with self.assertRaisesRegex(ValueError, 'original formal exit receipt'):
            self.retire()
        self.assert_targets(True)
        self.assertFalse((self.state/'handoff.json').exists())

    def test_absent_terminal_and_live_reference_cannot_authorize_handoff(self):
        self.enroll()
        with self.assertRaises(FileNotFoundError):
            self.retire()
        self.launch()
        with self.assertRaisesRegex(ValueError, 'in-use'):
            self.retire(side_effect=ValueError('in-use'))
        self.assert_targets(True)
        self.assertFalse((self.state/'handoff.json').exists())

    def test_target_must_be_new_and_may_not_hold_evidence(self):
        with self.assertRaisesRegex(ValueError, 'private formal build paths'):
            self.enroll(targets=('correct/source/.lake/extract-target',))
        with self.assertRaisesRegex(ValueError, 'private formal build paths'):
            self.enroll(targets=('module/original-native/other/target',))
        with self.assertRaisesRegex(ValueError, 'private formal build paths'):
            self.enroll(targets=('target',))
        self.enroll()
        self.launch()
        proof_inside_target = self.run/self.targets[0]/'build-output'
        with self.assertRaisesRegex(ValueError, 'inside a disposable build'):
            self.retire(readback=lambda _: {proof_inside_target: digest(proof_inside_target)})
        self.assert_targets(True)

    def test_retry_after_partial_move_uses_same_enrollment_and_inodes(self):
        self.enroll()
        self.launch()
        rename = Path.rename

        def interrupt(path, destination):
            if destination.name == '1':
                raise OSError('interrupted after first move')
            return rename(path, destination)

        with patch.object(Path, 'rename', interrupt):
            with self.assertRaisesRegex(OSError, 'interrupted'):
                self.retire()
        handoff = json_file(self.state/'handoff.json')
        moved = self.state/'store/payloads'/handoff['payload']/'0'
        self.assertEqual(native_task.directory_identity(moved), handoff['targets'][0]['identity'])
        self.assertEqual(self.retire()['target_count'], 2)
        self.assert_targets(False)


if __name__ == '__main__':
    unittest.main()
