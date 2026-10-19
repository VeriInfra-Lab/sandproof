"""Real filesystem checks for S03 evidence; no VM or package installation."""
import errno
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_guest_environment as environment


class ImmutableCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.directory = self.root / 'artifacts' / ('a' * 64)
        self.directory.mkdir(parents=True)
        self.disk = self.directory / 'rootfs.ext4'
        self.source = b'original immutable template'
        self.disk.write_bytes(self.source)
        self.disk.chmod(0o400)
        self.pin = dict(bytes=len(self.source), sha256=hashlib.sha256(self.source).hexdigest())
        local = dict(template_id='tpl-fixture', rootfs=dict(path=str(self.disk),
                    size_bytes=self.pin['bytes'], sha256=self.pin['sha256']))
        (self.directory / 'manifest.json').write_text(json.dumps(dict(local=local)))
        self.cache = dict(root=str(self.root), max_entries=1, max_entry_bytes=1024)

    def snapshot(self, pin=None):
        return environment.cached_template(self.cache, 'tpl-fixture', pin or self.pin)

    def test_actual_cached_bytes_and_original_descriptor_are_both_required(self):
        self.assertEqual(self.snapshot()['artifact'], self.pin)
        with self.assertRaises(AssertionError):
            self.snapshot(dict(bytes=self.pin['bytes'], sha256='0' * 64))
        self.disk.chmod(0o600)
        self.disk.write_bytes(b'X' * len(self.source))
        self.disk.chmod(0o400)
        with self.assertRaises(AssertionError):
            self.snapshot()

    def test_same_bytes_in_a_replacement_inode_are_distinct_observations(self):
        before = self.snapshot()
        replacement = self.directory / 'replacement'
        replacement.write_bytes(self.source)
        replacement.chmod(0o400)
        replacement.replace(self.disk)
        after = self.snapshot()
        self.assertEqual(before['artifact'], after['artifact'])
        self.assertNotEqual(before['identity'][:2], after['identity'][:2])

    def test_linked_or_symlinked_cache_file_is_not_original_evidence(self):
        link = self.directory / 'alias'
        link.hardlink_to(self.disk)
        with self.assertRaises(ValueError):
            self.snapshot()
        link.unlink()
        target = self.directory / 'target'
        self.disk.rename(target)
        self.disk.symlink_to(target)
        with self.assertRaises(OSError) as caught:
            self.snapshot()
        self.assertEqual(caught.exception.errno, errno.ELOOP)

    def test_guest_programs_compile_without_executing_host_mutations(self):
        for name in ('PRISTINE', 'PREPARE', 'IMPORTED'):
            compile(getattr(environment, name), '<guest-' + name + '>', 'exec')
