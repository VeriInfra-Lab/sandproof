import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_retention, native_task
from tools.system_acceptance import digest, json_file, write_once


class NativeTaskTests(unittest.TestCase):
    candidate = 'a' * 40

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.observer = self.root / 'observer'
        self.observer.write_text('test observer; calls intercepted below\n')
        self.output = self.root / 'native'
        native_task.enroll(self.output, self.candidate, 'stage1-protocol', (), self.observer)
        self.work = self.output / 'run-test'
        self.work.mkdir(mode=0o700)
        self.run = ci_retention.NativeBuildRun(self.work, self.candidate)
        self.source = self.work / 'source.rs'
        self.source.write_text('fn main() {}\n')
        self.shared = self.root / 'shared-cache'
        self.shared.mkdir()
        self.dependency = self.shared / 'dependency'
        self.dependency.write_text('retained shared dependency\n')

    def target(self, parent):
        path = self.work / parent / 'target'
        path.mkdir(mode=0o700, parents=True)
        (path / 'CACHEDIR.TAG').write_text('Signature: 8a477f597d28d172789f06886806bc55\n')
        (path / '.rustc_info.json').write_text('{}\n')
        (path / 'compiled').write_bytes(b'c' * 8192)
        return path

    def success(self):
        uses = [{'identity': 'fixture', 'sha256': 'b' * 64}]
        used = self.work / 'commands-used.jsonl'
        used.write_text(json.dumps(uses[0]) + '\n')
        used.chmod(0o600)
        original = self.work / 'native-complete.json'
        write_once(original, dict(complete=True, exit_code=0, scope_complete=True, uses=uses))
        self.run.finish(original)
        write_once(self.output / 'complete.json', json_file(original))
        return original

    def launch(self, code):
        return native_task.launch(self.output, [sys.executable, '-c', code])

    def retire(self, **kwargs):
        with native_task.task_lock(self.output), patch.object(native_task, 'observe', **kwargs):
            return native_task.retire(self.output, self.observer)

    def test_success_and_retry_keep_original_seal_and_caches(self):
        nested = [self.target('case'), self.target('retained')]
        original = self.success()
        before = digest(original), digest(self.source), digest(self.dependency)
        self.assertEqual(self.launch('print("actual exit")')['returncode'], 0)
        result = self.retire()
        self.assertEqual((result['status'], result['target_count']), ('passed', 2))
        self.assertEqual(result['remaining_build_bytes'], 0)
        self.assertGreaterEqual(result['allocated_bytes'], 2 * 8192)
        self.assertTrue(all(not path.exists() for path in nested))
        self.assertEqual((digest(original), digest(self.source), digest(self.dependency)), before)
        self.assertTrue((self.output / 'complete.json').exists())
        self.assertIn('actual exit', (self.output / 'native-task.log').read_text())
        self.assertEqual(self.retire(), result)

    def test_original_nonzero_exit_is_preserved(self):
        target = self.target('case')
        self.assertEqual(self.launch('import sys; sys.exit(19)')['returncode'], 19)
        result = self.retire()
        self.assertEqual((result['status'], result['returncode']), ('failed', 19))
        self.assertFalse(target.exists())
        self.assertFalse((self.output / 'complete.json').exists())

    def test_cancel_forwarding_and_original_signal(self):
        target = self.target('case')
        self.launch('import os, signal, time; os.kill(os.getppid(), signal.SIGTERM); time.sleep(5)')
        result = self.retire()
        self.assertEqual((result['status'], result['returncode']), ('cancelled', -signal.SIGTERM))
        self.assertFalse(target.exists())
        self.assertFalse((self.output / 'complete.json').exists())

    def test_unknown_process_reference_and_changed_observer_keep_targets(self):
        target = self.target('case')
        self.success()
        self.launch('pass')
        with self.assertRaisesRegex(ValueError, 'unknown reference'):
            self.retire(side_effect=ValueError('unknown reference'))
        self.assertTrue(target.is_dir())
        self.observer.write_text('replacement observer\n')
        with self.assertRaisesRegex(ValueError, 'same installed host observer'):
            self.retire()
        self.assertTrue(target.is_dir())

    def test_interrupted_handoff_resumes_same_target_identities(self):
        first = self.target('one')
        second = self.target('two')
        self.success()
        ordered = json_file(self.work / 'build-targets.json')['targets']
        self.launch('pass')
        rename = Path.rename

        def fail_second(target, destination):
            if Path(destination).name == '1':
                raise OSError('interrupted before second rename')
            return rename(target, destination)

        with patch.object(Path, 'rename', fail_second):
            with self.assertRaisesRegex(OSError, 'interrupted before second rename'):
                self.retire()
        handoff = self.work / 'build-handoff.json'
        before = digest(handoff)
        moved = Path(json_file(handoff)['payload']) / '0'
        self.assertEqual(native_task.directory_identity(moved), ordered[0]['identity'])
        result = self.retire()
        self.assertEqual(result['target_count'], 2)
        self.assertEqual(digest(handoff), before)
        self.assertFalse(moved.parent.exists())
        self.assertFalse(first.exists() or second.exists())

    def test_unenrolled_unknown_source_stays(self):
        target = self.target('case')
        (self.output / 'native-task.json').unlink()
        with self.assertRaises(FileNotFoundError):
            self.retire()
        self.assertTrue(target.is_dir())


if __name__ == '__main__':
    unittest.main()
