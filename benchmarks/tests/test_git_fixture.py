"""Fixture transport/extraction boundaries with synthetic bytes, no task execution."""

import hashlib
import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.guest_git_fixture import copy_verified, extract_packs


class GitFixtureTests(unittest.TestCase):
    def test_transport_rejects_truncation_extension_and_wrong_digest(self):
        expected = hashlib.sha256(b'abc').hexdigest()
        for payload in (b'ab', b'abcd', b'xyz'):
            with self.subTest(payload=payload), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    copy_verified(io.BytesIO(payload), Path(tmp)/'blob', 3, expected)

    def test_transport_never_overwrites_existing_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)/'blob'
            target.write_bytes(b'keep')
            with self.assertRaises(FileExistsError):
                copy_verified(io.BytesIO(b'abc'), target, 3, hashlib.sha256(b'abc').hexdigest())
            self.assertEqual(target.read_bytes(), b'keep')

    def extract(self, members, target):
        archive = target.parent/'layer.tar.gz'
        with tarfile.open(archive, 'w:gz') as stream:
            for name, payload, kind in members:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = len(payload)
                if kind == tarfile.SYMTYPE:
                    member.linkname = str(target.parent/'outside')
                stream.addfile(member, io.BytesIO(payload))
        files = {'fixture.pack': (3, hashlib.sha256(b'abc').hexdigest())}
        with patch('benchmarks.guest_git_fixture.PACK_FILES', files):
            extract_packs(archive, target)

    def test_only_pack_data_is_imported_and_image_code_is_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)/'packs';target.mkdir()
            self.extract([
                ('app/personal-site/.git/config', b'untrusted-config', tarfile.REGTYPE),
                ('../../outside', b'untrusted-code', tarfile.REGTYPE),
                ('app/personal-site/.git/objects/pack/fixture.pack', b'abc', tarfile.REGTYPE),
            ], target)
            self.assertEqual([p.name for p in target.iterdir()], ['fixture.pack'])
            self.assertEqual((target/'fixture.pack').read_bytes(), b'abc')
            self.assertFalse((Path(tmp)/'outside').exists())

    def test_missing_duplicate_symlink_and_corrupt_members_are_rejected(self):
        path = 'app/personal-site/.git/objects/pack/fixture.pack'
        valid = (path, b'abc', tarfile.REGTYPE)
        for members in ([], [valid, valid], [(path, b'abc', tarfile.SYMTYPE)],
                        [(path, b'ab', tarfile.REGTYPE)], [(path, b'xyz', tarfile.REGTYPE)]):
            with self.subTest(members=members), tempfile.TemporaryDirectory() as tmp:
                target = Path(tmp)/'packs';target.mkdir()
                with self.assertRaises(ValueError):
                    self.extract(members, target)


if __name__ == '__main__':
    unittest.main()
