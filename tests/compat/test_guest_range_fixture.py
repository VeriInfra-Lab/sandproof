"""Check the range fixture against original MIME header bytes, not email normalization."""
import unittest

import httpx

from guest_file_transfer_sdk import check_range


def response(content_type=b'text/plain; charset=utf-8', extra_header=b''):
    body = b''
    for offset, payload in [(0, b'n'), (2, b'w')]:
        body += (b'--fixture\r\nContent-Type: ' + content_type + b'\r\n'
                 + extra_header
                 + f'Content-Range: bytes {offset}-{offset}/3\r\n\r\n'.encode()
                 + payload + b'\r\n')
    body += b'--fixture--\r\n'
    return httpx.Response(206, headers={
        'content-type': 'multipart/byteranges; boundary=fixture',
        'content-length': str(len(body)),
    }, content=body)


class RangeFixtureHeaders(unittest.TestCase):
    def test_original_unquoted_charset_is_accepted(self):
        check_range(response(), 'GET', 206, None)

    def test_wrong_media_type_is_rejected(self):
        with self.assertRaises(AssertionError):
            check_range(response(b'application/octet-stream'), 'GET', 206, None)

    def test_wrong_charset_is_rejected(self):
        with self.assertRaises(AssertionError):
            check_range(response(b'text/plain; charset=utf-16'), 'GET', 206, None)

    def test_duplicate_content_type_is_rejected(self):
        with self.assertRaises(AssertionError):
            check_range(response(extra_header=b'Content-Type: text/plain; charset=utf-8\r\n'),
                        'GET', 206, None)

    def test_quoted_wire_charset_is_not_silently_normalized(self):
        with self.assertRaises(AssertionError):
            check_range(response(b'text/plain; charset="utf-8"'), 'GET', 206, None)


if __name__ == '__main__':
    unittest.main()
