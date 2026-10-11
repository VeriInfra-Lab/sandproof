import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import ci_redis as redis


class CiRedisMaterialTests(unittest.TestCase):
    def material(self, directory):
        binary = bytearray(64)
        binary[:6] = b'\x7fELF\x02\x01'
        binary[18:20] = b'\x3e\x00'
        (directory / 'redis-server').write_bytes(binary)
        receipt = dict(redis.PIN, binary_sha256=redis.digest(binary), binary_bytes=len(binary))
        (directory / 'material.json').write_text(json.dumps(receipt))
        return receipt

    def test_pin_matches_the_existing_authoritative_input_lock(self):
        baseline = json.loads((Path(__file__).resolve().parents[2] / 'references/v1-baseline.json').read_text())
        stored = baseline['code_baseline']['center_storage']
        self.assertEqual(redis.PIN, {'version': stored['redis_server'], 'archive_sha256': stored['archive_sha256']})

    def test_missing_corrupt_linked_wrong_type_and_oversized_material_refuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            good = self.material(directory)
            self.assertEqual(redis.check_material(directory), good)
            for field, value in (('version', 'latest'), ('archive_sha256', 'a' * 64),
                                 ('binary_sha256', 'a' * 64), ('binary_bytes', True), ('unknown', 'ignored')):
                bad = copy.deepcopy(good)
                bad[field] = value
                (directory / 'material.json').write_text(json.dumps(bad))
                with self.subTest(field=field), self.assertRaises(ValueError):
                    redis.check_material(directory)
            self.material(directory)
            (directory / 'redis-server').write_bytes(b'wrong')
            with self.assertRaises(ValueError):
                redis.check_material(directory)
            self.material(directory)
            with mock.patch.object(redis, 'BINARY_LIMIT', 32), self.assertRaises(ValueError):
                redis.check_material(directory)
            os.rename(directory / 'redis-server', directory / 'original')
            (directory / 'redis-server').symlink_to(directory / 'original')
            with self.assertRaises(OSError):
                redis.check_material(directory)
            (directory / 'redis-server').unlink()
            os.mkfifo(directory / 'redis-server')
            with self.assertRaises(ValueError):
                redis.check_material(directory)

    def test_archive_fingerprint_and_unprivileged_build_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            archive = directory / 'redis.tar.gz'
            archive.write_bytes(b'wrong source')
            with self.assertRaises(ValueError):
                redis.checked_archive(archive)
            with mock.patch.object(redis.os, 'geteuid', return_value=0), self.assertRaises(ValueError):
                redis.build(archive, directory)
            with mock.patch.object(redis.os, 'geteuid', return_value=1000), self.assertRaises(ValueError):
                redis.seal(directory)

    def test_seal_refuses_broad_cleanup_target_before_any_installation(self):
        with mock.patch.object(redis.os, 'geteuid', return_value=0), \
                mock.patch.object(redis, 'check_material') as check:
            for path in (Path('/'), Path('/var/tmp'), Path('/home/runner'), Path('/var/tmp/unrelated')):
                with self.subTest(path=path), self.assertRaises(ValueError):
                    redis.seal(path)
            check.assert_not_called()
