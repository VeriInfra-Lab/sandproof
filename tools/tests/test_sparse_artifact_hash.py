"""Exact full-byte digests, sparse reads, and fail-closed fallback boundaries."""
import errno
import hashlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import guest_artifacts as artifacts
from tools.template_stage import file_hash


class ArtifactDigestFallbackTests(unittest.TestCase):
    def test_dense_fallback_rewinds_and_hashes_empty_and_nonempty_full_files(self):
        for payload in (b'', b'hello', bytes(range(256))*512):
            with self.subTest(size=len(payload)), tempfile.TemporaryFile() as source:
                source.write(payload)
                source.seek(len(payload))
                with patch.object(artifacts, '_extent_digest', return_value=None):
                    self.assertEqual(artifacts.file_digest(source, len(payload)),
                                     hashlib.sha256(payload).hexdigest())

    def test_dense_fallback_rejects_truncation_growth_and_real_read_errors(self):
        for payload, length in ((b'a', 2), (b'ab', 1)):
            with self.subTest(payload=payload), tempfile.TemporaryFile() as source:
                source.write(payload)
                with patch.object(artifacts, '_extent_digest', return_value=None):
                    with self.assertRaisesRegex(ValueError, 'artifact .* while reading'):
                        artifacts.file_digest(source, length)
        class Broken(io.BytesIO):
            def fileno(self):
                return 99
            def read(self, count):
                raise OSError(errno.EIO, 'injected read failure')
        with patch.object(artifacts, '_extent_digest', return_value=None):
            with self.assertRaises(OSError):
                artifacts.file_digest(Broken(b'a'), 1)


class LinuxSparseArtifactHashTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name)/'artifact'
        self.path.touch(mode=0o600)

    def sparse(self, size=8*1024**2+17):
        with self.path.open('r+b') as source:
            source.write(b'head')
            source.seek(size//2)
            source.write(b'middle')
            source.truncate(size)
        expected = bytearray(size)
        expected[:4] = b'head'
        expected[size//2:size//2+6] = b'middle'
        return bytes(expected)

    def test_real_holes_contribute_zero_bytes_without_disk_reads(self):
        expected = self.sparse()
        calls = []
        pread = os.pread
        def observed(fd, count, offset):
            calls.append((count, offset))
            return pread(fd, min(count, 997), offset)
        with patch.object(artifacts.os, 'pread', observed):
            pin = artifacts.fingerprint(self.path, len(expected))
            self.assertEqual(file_hash(self.path), hashlib.sha256(expected).hexdigest())
        self.assertEqual(pin, dict(bytes=len(expected), sha256=hashlib.sha256(expected).hexdigest()))
        self.assertTrue(calls)
        self.assertTrue(all(count <= 65536 for count, _ in calls))
        self.assertLess(sum(count for count, _ in calls), len(expected)//4)
        # Returning content still observes every actual byte for its consumer.
        self.assertEqual(artifacts.fingerprint(self.path, len(expected), content=True), (pin, expected))

    def test_empty_and_all_hole_files_never_read_but_keep_full_logical_hash(self):
        for size in (0, 1, 131075):
            with self.subTest(size=size):
                with self.path.open('wb') as source:
                    source.truncate(size)
                with patch.object(artifacts.os, 'pread', side_effect=AssertionError('hole read')):
                    self.assertEqual(artifacts.fingerprint(self.path, size, allow_empty=True),
                                     dict(bytes=size, sha256=hashlib.sha256(bytes(size)).hexdigest()))
                    self.assertEqual(file_hash(self.path), hashlib.sha256(bytes(size)).hexdigest())

    def test_unsupported_extent_queries_after_partial_progress_restart_the_full_hash(self):
        expected = self.sparse(131075)
        seek = os.lseek
        for when in (1, 2, 3, 4):
            for code in (errno.EINVAL, errno.ENOSYS, errno.EOPNOTSUPP):
                count = 0
                def unsupported(fd, offset, whence):
                    nonlocal count
                    count += 1
                    if count == when:
                        raise OSError(code, 'unsupported extent')
                    return seek(fd, offset, whence)
                with self.subTest(when=when, code=code), patch.object(artifacts.os, 'lseek', unsupported):
                    self.assertEqual(file_hash(self.path), hashlib.sha256(expected).hexdigest())
                self.assertGreaterEqual(count, when)

    def test_real_seek_and_read_errors_are_never_dense_fallback(self):
        self.sparse(131075)
        for code in (errno.EIO, errno.EACCES):
            with self.subTest(seek_error=code), patch.object(artifacts.os, 'lseek', side_effect=OSError(code, 'seek failed')):
                with self.assertRaises(OSError) as caught:
                    file_hash(self.path)
                self.assertEqual(caught.exception.errno, code)
        for code in (errno.EIO, errno.EINVAL, errno.ENOSYS, errno.EOPNOTSUPP):
            with self.subTest(read_error=code), patch.object(artifacts.os, 'pread', side_effect=OSError(code, 'read failed')):
                with self.assertRaises(OSError) as caught:
                    file_hash(self.path)
                self.assertEqual(caught.exception.errno, code)

    def test_malformed_extents_and_empty_data_reads_fail(self):
        self.path.write_bytes(b'abcd')
        for extents in ((-1,), (4,), (0, 0), (0, -1)):
            with self.subTest(extents=extents), patch.object(artifacts.os, 'lseek', side_effect=extents):
                with self.assertRaisesRegex(ValueError, 'invalid artifact .* extent'):
                    file_hash(self.path)
        with patch.object(artifacts.os, 'pread', return_value=b''):
            with self.assertRaisesRegex(ValueError, 'changed while reading'):
                file_hash(self.path)

    def test_same_size_replacement_and_length_changes_still_fail_fingerprint(self):
        for change in ('replace', 'truncate', 'grow'):
            self.path.write_bytes(b'original')
            pread = os.pread
            changed = False
            def mutate(fd, count, offset):
                nonlocal changed
                block = pread(fd, count, offset)
                if not changed:
                    changed = True
                    if change == 'replace':
                        other = self.path.with_name('replacement')
                        other.write_bytes(b'original')
                        other.chmod(0o600)
                        other.replace(self.path)
                    else:
                        with self.path.open('r+b') as output:
                            output.truncate(1 if change == 'truncate' else 16)
                return block
            with self.subTest(change=change), patch.object(artifacts.os, 'pread', mutate):
                with self.assertRaises(ValueError):
                    artifacts.fingerprint(self.path, 16)
            self.assertTrue(changed)


if __name__ == '__main__':
    unittest.main()
