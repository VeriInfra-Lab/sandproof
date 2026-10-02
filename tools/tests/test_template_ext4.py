"""Candidate population failures must still reclaim the private guest mount."""
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.template_worker import Commands, populate_ext4


class Ext4PopulationTests(unittest.TestCase):
    def exercise(self, *, fail=None, mounted=True, cleanup_fails=False):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        disk, root = directory / 'candidate.ext4', directory / 'rootfs'
        root.mkdir()
        active, events, now = [False], [], [100.0]
        commands = Commands(110.0, io.BytesIO(), 4096)
        original = TimeoutError('original population failure')

        def run(runner, args, **kwargs):
            tool = Path(args[0]).name
            events.append(tool)
            if tool == 'mount':
                active[0] = mounted
            if tool == fail:
                now[0] = 111.0  # Cleanup must remain possible after expiry.
                raise original
            if tool == 'umount':
                self.assertGreater(runner.deadline, now[0])
                self.assertIs(runner.budget, commands.budget)
                self.assertIs(runner.log, commands.log)
                if cleanup_fails:
                    raise RuntimeError('cleanup failure')
                active[0] = False

        with patch('tools.template_worker.time.monotonic', side_effect=lambda: now[0]), \
             patch('tools.template_worker.os.path.ismount', side_effect=lambda p: active[0]), \
             patch.object(Commands, 'run', run):
            error = None
            try:
                populate_ext4(commands, root, disk)
            except Exception as caught:
                error = caught
        return error, original, events, active[0], directory / 'candidate-mount'

    def test_success_reclaims_mount_before_returning(self):
        error, _, events, active, mount = self.exercise()
        self.assertIsNone(error)
        self.assertEqual(events[-1], 'umount')
        self.assertFalse(active)
        self.assertFalse(mount.exists())

    def test_expired_copy_failure_still_reclaims_mount_without_replacing_error(self):
        error, original, events, active, mount = self.exercise(fail='cp')
        self.assertIs(error, original)
        self.assertEqual(events[-1], 'umount')
        self.assertFalse(active)
        self.assertFalse(mount.exists())

    def test_mount_reporting_failure_after_mounting_still_gets_reclaimed(self):
        error, original, events, active, _ = self.exercise(fail='mount')
        self.assertIs(error, original)
        self.assertNotIn('cp', events)
        self.assertEqual(events[-1], 'umount')
        self.assertFalse(active)

    def test_missing_mount_cannot_copy_into_the_builder_filesystem(self):
        error, _, events, active, _ = self.exercise(mounted=False)
        self.assertIsInstance(error, RuntimeError)
        self.assertNotIn('cp', events)
        self.assertFalse(active)

    def test_cleanup_failure_prevents_success(self):
        error, _, _, active, mount = self.exercise(cleanup_fails=True)
        self.assertIsInstance(error, RuntimeError)
        self.assertTrue(active)
        self.assertTrue(mount.exists())

    def test_cleanup_failure_keeps_original_failure_and_records_reclaim_need(self):
        error, original, _, active, _ = self.exercise(fail='cp', cleanup_fails=True)
        self.assertIs(error, original)
        self.assertEqual(error.__notes__, ['candidate mount cleanup failed; external VM cleanup required'])
        self.assertTrue(active)


if __name__ == '__main__':
    unittest.main()
