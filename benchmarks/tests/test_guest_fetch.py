"""Archive integrity regressions; no external downloads or task execution."""
import hashlib
import io
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.guest_fetch import fetch


class RangeResponse(io.BytesIO):
    def __init__(self, data, start, end, total, status=206):
        super().__init__(data)
        self.status = status
        self.headers = {'Content-Range': f'bytes {start}-{end}/{total}'}


class GuestFetchTests(unittest.TestCase):
    payload = b'archive-with-several-ranges'

    def response(self, request, timeout):
        start, end = map(int, re.fullmatch(r'bytes=(\d+)-(\d+)', request.get_header('Range')).groups())
        return RangeResponse(self.payload[start:end + 1], start, end, len(self.payload))

    def run_download(self, output, response=None, digest=None):
        with patch('benchmarks.guest_fetch.CHUNK_BYTES', 4), patch(
                'benchmarks.guest_fetch.urllib.request.urlopen', side_effect=response or self.response):
            return fetch('https://example.invalid/archive', output, len(self.payload),
                         digest or hashlib.sha256(self.payload).hexdigest())

    def test_exact_ranges_reconstruct_verified_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'archive'
            result = self.run_download(output)
            self.assertEqual(output.read_bytes(), self.payload)
            self.assertEqual(result['sha256'], hashlib.sha256(self.payload).hexdigest())
            self.assertEqual(list(Path(tmp).iterdir()), [output])

    def test_hash_mismatch_never_publishes_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                self.run_download(Path(tmp) / 'archive', digest='0' * 64)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_truncated_range_is_rejected(self):
        def truncated(request, timeout):
            r = self.response(request, timeout)
            r.truncate(len(r.getvalue()) - 1)
            return r
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'truncated range'):
                self.run_download(Path(tmp) / 'archive', truncated)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_ignored_range_is_rejected(self):
        def ignored(request, timeout):
            r = self.response(request, timeout)
            r.status = 200
            return r
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'exact requested range'):
                self.run_download(Path(tmp) / 'archive', ignored)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_existing_file_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp) / 'archive'
            output.write_bytes(b'existing')
            with self.assertRaises(FileExistsError):
                self.run_download(output)
            self.assertEqual(output.read_bytes(), b'existing')

    def test_unsafe_or_unbounded_inputs_are_rejected(self):
        for url, size, digest in (
            ('http://example.invalid/archive', 1, '0' * 64),
            ('https://example.invalid/archive', 0, '0' * 64),
            ('https://example.invalid/archive', 2 * 1024**3, '0' * 64),
            ('https://example.invalid/archive', 1, 'bad'),
        ):
            with self.subTest(url=url, size=size, digest=digest), self.assertRaises(ValueError):
                fetch(url, '/unused', size, digest)


if __name__ == '__main__':
    unittest.main()
