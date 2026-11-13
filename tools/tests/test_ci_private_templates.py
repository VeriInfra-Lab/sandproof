import json
from pathlib import Path
import tempfile
import unittest

from tools import ci_private_templates as templates


class PrivateTemplateTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name).resolve()
        self.cache=self.root/'cache';self.cache.mkdir(mode=0o700)
        (self.cache/'inbox').mkdir(mode=0o700)
        (self.cache/'artifacts').mkdir(mode=0o700)
        self.entry=self.cache/'artifacts'/('a'*64);self.entry.mkdir(mode=0o700)
        for name in ('rootfs.ext4','kernel','manifest.json'):
            p=self.entry/name;p.write_bytes(b'original');p.chmod(0o600)
        self.record=templates.cache_identity(self.cache)
        self.evidence=self.root/'retirement.json'

    def test_only_private_data_removed_and_manifest_remains(self):
        shared=self.root/'shared-template';shared.write_bytes(b'untouched')
        unknown=self.root/'unknown-volume';unknown.write_bytes(b'untouched')
        templates.retire([self.record],self.evidence)
        self.assertFalse((self.entry/'rootfs.ext4').exists())
        self.assertFalse((self.entry/'kernel').exists())
        self.assertEqual((self.entry/'manifest.json').read_bytes(),b'original')
        self.assertEqual(shared.read_bytes(),b'untouched')
        self.assertEqual(unknown.read_bytes(),b'untouched')
        self.assertEqual(json.loads(self.evidence.read_text())['files'][0]['manifest_sha256'],
                         templates.digest(self.entry/'manifest.json'))

    def test_replaced_cache_or_unfinished_download_retains_files(self):
        self.record['identity']=[0,0]
        with self.assertRaises(ValueError):templates.retire([self.record],self.evidence)
        self.record=templates.cache_identity(self.cache)
        (self.cache/'inbox'/'partial').write_text('unfinished')
        with self.assertRaises(ValueError):templates.retire([self.record],self.evidence)
        self.assertTrue((self.entry/'rootfs.ext4').exists())
        self.assertFalse(self.evidence.exists())

    def test_link_to_shared_data_rejected_before_first_deletion(self):
        shared=self.root/'shared';shared.write_bytes(b'original')
        (self.entry/'kernel').unlink();(self.entry/'kernel').symlink_to(shared)
        with self.assertRaises(ValueError):templates.retire([self.record],self.evidence)
        self.assertTrue((self.entry/'rootfs.ext4').exists())
        self.assertEqual(shared.read_bytes(),b'original')

    def test_evidence_inside_cache_or_existing_receipt_refuses_deletion(self):
        with self.assertRaises(ValueError):templates.retire([self.record],self.cache/'report.json')
        self.evidence.write_text('old')
        with self.assertRaises(FileExistsError):templates.retire([self.record],self.evidence)
        self.assertTrue((self.entry/'rootfs.ext4').exists())
