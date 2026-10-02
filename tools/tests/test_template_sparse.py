import errno
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.template_build_store import BuildError, durable_write
from tools.template_stage import copy_sparse, file_hash, stage


class SparseArtifactTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.path = self.root / 'candidate.ext4'

    def test_stream_boundaries_middle_and_trailing_zeros_keep_bytes_length_and_digest(self):
        payload = b'head' + bytes(1024 * 1024 + 17) + b'center' + bytes(1024 * 1024 + 3)
        for chunk in (4093, 65536, 1048576):
            with self.subTest(chunk=chunk):
                count, digest = durable_write(self.path,
                    (payload[i:i + chunk] for i in range(0, len(payload), chunk)),
                    len(payload), exact=len(payload), sparse=True)
                self.assertEqual(count, len(payload))
                self.assertEqual(digest, hashlib.sha256(payload).hexdigest())
                self.assertEqual(self.path.read_bytes(), payload)
                self.assertEqual(self.path.stat().st_size, count)
                if hasattr(self.path.stat(), 'st_blocks'):
                    self.assertLess(self.path.stat().st_blocks * 512, len(payload) // 4)
                self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)

    def test_empty_and_all_zero_files_extend_to_exact_logical_length(self):
        for size in (0, 1, 131075):
            with self.subTest(size=size):
                count, digest = durable_write(self.path, [b'', bytes(size), b''], size, exact=size, sparse=True)
                self.assertEqual(self.path.stat().st_size, size)
                self.assertEqual(count, size)
                self.assertEqual(digest, hashlib.sha256(bytes(size)).hexdigest())
                self.assertEqual(self.path.read_bytes(), bytes(size))

    def test_zeros_still_consume_limit_and_incomplete_input_never_replaces_old_target(self):
        self.path.write_bytes(b'previous accepted bytes')
        for blocks, limit, exact, reason in (([bytes(4097)], 4096, None, 'TooLarge'),
                                            ([bytes(4095)], 4096, 4096, 'Incomplete')):
            with self.subTest(reason=reason), self.assertRaisesRegex(BuildError, reason):
                durable_write(self.path, blocks, limit, exact=exact, sparse=True)
            self.assertEqual(self.path.read_bytes(), b'previous accepted bytes')
            self.assertFalse(self.path.with_suffix('.partial').exists())

    def test_interrupted_stream_and_fsync_failure_remove_only_unpublished_partial(self):
        self.path.write_bytes(b'old artifact')
        def interrupted():
            yield b'head'
            yield bytes(1024 * 1024)
            raise OSError(errno.EIO, 'injected transport interruption')
        with self.assertRaises(OSError):
            durable_write(self.path, interrupted(), 2 * 1024 * 1024, sparse=True)
        with patch('tools.template_build_store.os.fsync', side_effect=OSError(errno.ENOSPC, 'full')):
            with self.assertRaises(OSError):
                durable_write(self.path, [b'head', bytes(1024)], 1028, exact=1028, sparse=True)
        self.assertEqual(self.path.read_bytes(), b'old artifact')
        self.assertFalse(self.path.with_suffix('.partial').exists())

    def test_stale_partial_is_truncated_before_zero_holes_are_created(self):
        self.path.with_suffix('.partial').write_bytes(b'old' * 65536)
        durable_write(self.path, [bytes(65536)], 65536, exact=65536, sparse=True)
        self.assertEqual(self.path.read_bytes(), bytes(65536))

    def test_normal_binary_data_is_not_changed_and_existing_copy_target_is_not_overwritten(self):
        source = self.root / 'source'
        payload = bytes(range(256)) * 521 + b'end'
        source.write_bytes(payload)
        copy_sparse(source, self.path)
        self.assertEqual(self.path.read_bytes(), payload)
        self.assertEqual(source.read_bytes(), payload)
        with self.assertRaises(FileExistsError):
            copy_sparse(source, self.path)
        self.assertEqual(self.path.read_bytes(), payload)

    def test_extent_short_copies_and_fallback_after_partial_copy_preserve_bytes(self):
        source = self.root / 'source'
        size = 2 * 1024 * 1024 + 19
        with source.open('wb') as output:
            output.write(b'head' * 1024)
            output.seek(size // 2)
            output.write(b'center' * 1024)
            output.truncate(size)
        expected = source.read_bytes()
        original_copy = os.sendfile
        for failure in (None, errno.EXDEV, errno.ENOSYS, errno.EOPNOTSUPP, 0):
            calls = []
            def short_copy(dst, src, offset, count):
                calls.append(count)
                if len(calls) > 1 and failure is not None:
                    if failure == 0:
                        return 0
                    raise OSError(failure, 'injected unsupported copy')
                return original_copy(dst, src, offset, min(count, 1024))
            with self.subTest(failure=failure), patch('tools.template_stage.os.sendfile', short_copy):
                target = self.root / ('copy-' + str(failure))
                copy_sparse(source, target)
                self.assertEqual(target.read_bytes(), expected)
                self.assertNotEqual(target.stat().st_ino, source.stat().st_ino)
                self.assertLess(target.stat().st_blocks * 512, size // 4)
                self.assertGreater(len(calls), 1)
            self.assertEqual(source.read_bytes(), expected)

    def test_unsupported_extent_lookup_and_all_hole_sources_keep_exact_length(self):
        source = self.root / 'source'
        for payload in (b'', bytes(131075), b'head' + bytes(131075) + b'tail'):
            with source.open('wb') as output:
                if payload.strip(b'\0'):
                    output.write(payload)
                else:
                    output.truncate(len(payload))
            for unsupported in (False, True):
                with self.subTest(size=len(payload), unsupported=unsupported):
                    self.path.unlink(missing_ok=True)
                    if unsupported:
                        with patch('tools.template_stage.os.lseek', side_effect=OSError(errno.EINVAL, 'unsupported')):
                            copy_sparse(source, self.path)
                    else:
                        copy_sparse(source, self.path)
                    self.assertEqual(self.path.read_bytes(), payload)
                    self.assertEqual(self.path.stat().st_size, len(payload))

    def test_real_io_errors_and_source_size_changes_do_not_succeed(self):
        source = self.root / 'source'
        source.write_bytes(b'payload' * 4096)
        for error in (errno.EIO, errno.ENOSPC):
            with self.subTest(error=error), patch('tools.template_stage.os.sendfile',
                    side_effect=OSError(error, 'injected real copy failure')):
                with self.assertRaises(OSError) as raised:
                    copy_sparse(source, self.root / ('error-' + str(error)))
                self.assertEqual(raised.exception.errno, error)
        original_copy = os.sendfile
        for change in (-1, 1):
            source.write_bytes(b'payload' * 4096)
            size = source.stat().st_size
            def changed_copy(dst, src, offset, count):
                result = original_copy(dst, src, offset, count)
                with source.open('r+b') as output:
                    output.truncate(size + change)
                return result
            with self.subTest(change=change), patch('tools.template_stage.os.sendfile', changed_copy):
                with self.assertRaisesRegex(OSError, 'source size changed'):
                    copy_sparse(source, self.root / ('changed-' + str(change)))

    def test_staging_records_full_shape_checks_digest_and_recovers_same_candidate(self):
        registry = self.root / 'registry'
        for p in (registry, registry / 'inbox', registry / 'artifacts'):
            p.mkdir(mode=0o700)
        kernel = self.root / 'kernel'
        kernel.write_bytes(b'kernel')
        size = 8 * 1024 * 1024
        with self.path.open('wb') as output:
            output.write(b'head')
            output.seek(size // 2)
            output.write(b'center')
            output.truncate(size)
        status = {'state': 'candidate', 'ready': False, 'startup_applied': False,
                  'disk_bytes': size, 'min_free_bytes': 1, 'image_digest': 'sha256:' + 'a' * 64,
                  'rootfs_sha256': file_hash(self.path),
                  'runtime_defaults': {'env_vars': {}, 'user': 'root', 'workdir': '/', 'argv': []}}
        identity = 'a' * 32
        partial = registry / 'inbox' / ('.stage-' + identity)
        partial.mkdir(mode=0o700)
        (partial / 'rootfs.ext4').write_bytes(b'interrupted')
        args = (registry, kernel, self.path, status, 1, 1, 256, 384)
        self.assertEqual(stage(*args, candidate_id=identity), identity)
        target = registry / 'inbox' / identity / 'rootfs.ext4'
        self.assertEqual(file_hash(target), status['rootfs_sha256'])
        self.assertEqual(target.stat().st_size, size)
        if hasattr(target.stat(), 'st_blocks'):
            self.assertLess(target.stat().st_blocks * 512, size // 4)
        manifest = json.loads((target.parent / 'manifest.json').read_text())
        self.assertEqual(manifest['binding']['rootfs']['size_bytes'], size)
        self.assertEqual(manifest['binding']['resources']['disk_bytes'], size)
        self.assertEqual(target.stat().st_mode & 0o777, 0o444)
        self.assertEqual(stage(*args, candidate_id=identity), identity)
        self.assertFalse(partial.exists())
        target.chmod(0o600)
        with target.open('r+b') as output:
            output.seek(size - 1)
            output.write(b'!')
        target.chmod(0o444)
        with self.assertRaisesRegex(ValueError, 'conflict'):
            stage(*args, candidate_id=identity)


if __name__ == '__main__':
    unittest.main()
