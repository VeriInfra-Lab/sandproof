import errno
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from tools.template_stage import file_hash, stage_downloaded


class DownloadHandoffTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.identity = 'a' * 32
        self.root = self.base / 'registry'
        self.slot = self.base / self.identity
        for path in (self.root, self.root / 'inbox', self.root / 'artifacts', self.slot):
            path.mkdir(mode=0o700)
        self.disk = self.slot / 'candidate.ext4'
        self.disk.write_bytes(b'head' + bytes(131075) + b'tail')
        self.disk.chmod(0o600)
        self.kernel = self.base / 'kernel'
        self.kernel.write_bytes(b'kernel')
        self.original_inode = self.disk.stat().st_ino
        self.original_hash = file_hash(self.disk)
        self.status = dict(state='candidate', ready=False, startup_applied=False,
                           disk_bytes=self.disk.stat().st_size, min_free_bytes=1,
                           image_digest='sha256:' + 'b' * 64, rootfs_sha256=self.original_hash,
                           runtime_defaults=dict(env_vars={}, user='root', workdir='/', argv=[]))
        self.args = (self.root, self.kernel, self.disk, self.status, 1, 1, 256, 384)
        self.pending = self.root / 'inbox' / ('.stage-' + self.identity)
        self.final = self.root / 'inbox' / self.identity

    def stage(self):
        return stage_downloaded(*self.args, candidate_id=self.identity)

    def assert_complete(self):
        target = self.final / 'rootfs.ext4'
        self.assertEqual(file_hash(target), self.original_hash)
        self.assertEqual(target.stat().st_nlink, 1)
        self.assertEqual(target.stat().st_mode & 0o777, 0o444)
        self.assertFalse(self.pending.exists())
        self.assertEqual(list((self.root / 'artifacts').iterdir()), [])

    def test_same_filesystem_consumes_slot_and_retry_checks_immutable_manifest(self):
        self.assertEqual(self.stage(), self.identity)
        self.assertFalse(self.disk.exists())
        self.assertEqual((self.final / 'rootfs.ext4').stat().st_ino, self.original_inode)
        self.assert_complete()
        with patch('tools.template_stage.copy_sparse', side_effect=AssertionError('unexpected copy')):
            self.assertEqual(self.stage(), self.identity)
        manifest_path = self.final / 'manifest.json'
        manifest = json.loads(manifest_path.read_text())
        manifest['binding']['owner'] = 2
        manifest_path.chmod(0o600)
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'conflict'):
            self.stage()
        self.assertTrue(self.final.exists())

    def test_cross_device_fallback_copies_without_consuming_source(self):
        rename = os.rename
        def cross_device(source, target):
            if source == self.disk:
                raise OSError(errno.EXDEV, 'different filesystems')
            return rename(source, target)
        with patch('tools.template_stage.os.rename', cross_device):
            self.stage()
        self.assertEqual(file_hash(self.disk), self.original_hash)
        self.assertNotEqual((self.final / 'rootfs.ext4').stat().st_ino, self.original_inode)
        self.assert_complete()
        self.assertEqual(self.stage(), self.identity)

    def test_process_death_after_disk_or_final_rename_recovers_same_inode(self):
        script = '''
import json, os, sys
from pathlib import Path
from tools.template_stage import stage_downloaded
args = json.loads(sys.argv[1])
args[:3] = map(Path, args[:3])
identity, crash_source = sys.argv[2:]
rename = os.rename
def crash_after_rename(source, target):
    rename(source, target)
    if str(source) == crash_source:
        os._exit(67)
os.rename = crash_after_rename
stage_downloaded(*args, candidate_id=identity)
'''
        for crash_source in (self.disk, self.pending):
            with self.subTest(crash_source=crash_source):
                if self.final.exists():
                    os.rename(self.final / 'rootfs.ext4', self.disk)
                    self.disk.chmod(0o600)
                    (self.final / 'kernel').unlink()
                    (self.final / 'manifest.json').unlink()
                    self.final.rmdir()
                args = [str(value) if isinstance(value, Path) else value for value in self.args]
                result = subprocess.run([sys.executable, '-c', script, json.dumps(args),
                                         self.identity, str(crash_source)], check=False, timeout=15)
                self.assertEqual(result.returncode, 67)
                self.assertFalse(self.disk.exists())
                self.assertEqual(self.stage(), self.identity)
                self.assertEqual((self.final / 'rootfs.ext4').stat().st_ino, self.original_inode)
                self.assert_complete()

    def test_failed_readback_restores_download_without_touching_another_identity(self):
        unrelated = self.root / 'inbox' / ('c' * 32)
        unrelated.mkdir(mode=0o700)
        (unrelated / 'keep').write_bytes(b'other')
        def corrupt_digest(path):
            return '0' * 64 if path == self.pending / 'rootfs.ext4' else file_hash(path)
        with patch('tools.template_stage.file_hash', corrupt_digest):
            with self.assertRaisesRegex(ValueError, 'changed during staging'):
                self.stage()
        self.assertEqual(self.disk.stat().st_ino, self.original_inode)
        self.assertEqual(file_hash(self.disk), self.original_hash)
        self.assertFalse(self.pending.exists())
        self.assertFalse(self.final.exists())
        self.assertEqual((unrelated / 'keep').read_bytes(), b'other')
        self.assertEqual(self.stage(), self.identity)

    def test_failed_source_directory_sync_restores_slot(self):
        from tools.template_stage import sync_directory
        failed = False
        def fail_once(path):
            nonlocal failed
            if path == self.slot and not failed:
                failed = True
                raise OSError(errno.EIO, 'injected directory sync failure')
            sync_directory(path)
        with patch('tools.template_stage.sync_directory', fail_once):
            with self.assertRaises(OSError):
                self.stage()
        self.assertTrue(failed)
        self.assertEqual(file_hash(self.disk), self.original_hash)
        self.assertFalse(self.pending.exists())
        self.assertEqual(self.stage(), self.identity)

    def test_shared_or_writable_by_others_download_is_never_consumed(self):
        alias = self.slot / 'alias'
        os.link(self.disk, alias)
        with self.assertRaisesRegex(ValueError, 'single-link'):
            self.stage()
        alias.unlink()
        self.disk.chmod(0o666)
        with self.assertRaisesRegex(ValueError, 'single-link'):
            self.stage()
        self.disk.chmod(0o600)
        self.assertEqual(self.disk.stat().st_ino, self.original_inode)
        self.assertEqual(list((self.root / 'inbox').iterdir()), [])
        with self.assertRaisesRegex(ValueError, 'fixed supervisor'):
            stage_downloaded(*self.args, candidate_id='d' * 32)
        self.slot.chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'private operator'):
            self.stage()
        self.assertEqual(file_hash(self.disk), self.original_hash)


if __name__ == '__main__':
    unittest.main()
