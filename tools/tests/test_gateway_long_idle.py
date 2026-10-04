"""Real local TCP/framing and failure classification; never native acceptance."""
import asyncio
from contextlib import contextmanager
import hashlib
import importlib.util
from io import BytesIO
from http.client import HTTPResponse
import os
from pathlib import Path
import socket
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/gateway_long_idle.py'
SPEC = importlib.util.spec_from_file_location('gateway_long_probe', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


@contextmanager
def loopback():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        listener.listen()
        client = probe.Transfer('http://127.0.0.1:' + str(listener.getsockname()[1]), 'abcd', small_window=True)
        server, _ = listener.accept()
        server.settimeout(3)
        try:
            yield client, server
        finally:
            client.close()
            server.close()


def response_transfer(wire):
    class Socket:
        def makefile(self, *_args):
            return BytesIO(wire)
    transfer = probe.Transfer.__new__(probe.Transfer)
    transfer.response = HTTPResponse(Socket())
    transfer.response.begin()
    transfer.total, transfer.sha, transfer.started = 0, hashlib.sha256(), probe.time.monotonic()
    return transfer


class FramingTests(unittest.TestCase):
    def test_chunked_truncation_and_complete_message_are_distinguished(self):
        head = b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n'
        with patch.object(probe, 'DOWNLOAD_BYTES', 6):
            transfer = response_transfer(head + b'6\r\nXXXXXX\r\n0\r\n\r\n')
            result = transfer.finish(truncated=False)
            self.assertEqual(result['sha256'], hashlib.sha256(b'XXXXXX').hexdigest())
            transfer = response_transfer(head + b'2\r\nXX\r\n4\r\n')
            self.assertEqual(transfer.finish(truncated=True)['bytes'], 2)
            for wire, truncated in ((head + b'2\r\nXX\r\n4\r\n', False),
                                    (head + b'6\r\nXXXXXX\r\n0\r\n\r\n', True),
                                    (head + b'2\r\nXX\r\n0\r\n\r\n', True)):
                with self.subTest(wire=wire, truncated=truncated), self.assertRaises(AssertionError):
                    response_transfer(wire).finish(truncated=truncated)

    def test_content_length_eof_must_leave_unfulfilled_length(self):
        with patch.object(probe, 'DOWNLOAD_BYTES', 6):
            transfer = response_transfer(b'HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nXX')
            self.assertTrue(transfer.finish(truncated=True)['incomplete_http_body'])
            with self.assertRaises(AssertionError):
                response_transfer(b'HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nXXXXXX').finish(truncated=True)

    def test_timeout_reset_corruption_and_excess_never_count_as_expiry(self):
        for error in (TimeoutError(), ConnectionResetError(), OSError()):
            transfer = response_transfer(b'HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nXX')
            transfer.response = SimpleNamespace(read=lambda _size, error=error: (_ for _ in ()).throw(error))
            with self.assertRaises(type(error)):
                transfer.finish(truncated=True)
        with patch.object(probe, 'DOWNLOAD_BYTES', 6):
            for body in (b'XXYXXX', b'XXXXXXX'):
                with self.assertRaises(AssertionError):
                    response_transfer(b'HTTP/1.1 200 OK\r\nContent-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body).finish(truncated=False)

    def test_stat_json_accepts_protobuf_int64_and_rejects_wrong_shape(self):
        import json
        for size in (probe.DOWNLOAD_BYTES, str(probe.DOWNLOAD_BYTES)):
            probe.assert_metadata(dict(status=200, body=json.dumps(dict(entry=dict(size=size)))))
        for body in (dict(size=probe.DOWNLOAD_BYTES), dict(entry=dict(size=1)), dict(entry=dict(size=True))):
            with self.assertRaises(AssertionError):
                probe.assert_metadata(dict(status=200, body=json.dumps(body)))

    def test_only_original_deadline_408_is_expiry(self):
        probe.assert_expiry(13000, 408)
        probe.assert_expiry(13005, 408)
        for elapsed, status in ((30, 408), (12999.9, 408), (13181, 408), (13001, 504), (13001, 200)):
            with self.subTest(elapsed=elapsed, status=status), self.assertRaises(AssertionError):
                probe.assert_expiry(elapsed, status)


@unittest.skipUnless(sys.platform == 'linux', 'Linux TCP_INFO and proc socket identity')
class TCPTests(unittest.TestCase):
    def test_kernel_counter_and_exact_server_socket_ownership(self):
        with loopback() as (client, server):
            before = probe.received_bytes(client.sock)
            server.sendall(b'X' * 1024)
            self.assertEqual(client.sock.recv(1024, socket.MSG_WAITALL), b'X' * 1024)
            self.assertEqual(probe.received_bytes(client.sock) - before, 1024)
            snapshot = probe.peer_snapshot(client.sock, os.getpid())
            self.assertTrue(snapshot['owned'])
            self.assertEqual(snapshot['state'], '01')
            server.close()
            self.assertFalse(probe.peer_snapshot(client.sock, os.getpid())['owned'])

    def test_close_interrupts_pending_metadata_reader(self):
        with loopback() as (client, _server):
            errors = []
            entered = threading.Event()
            def read():
                entered.set()
                try:
                    client.metadata()
                except Exception as error:
                    errors.append(type(error).__name__)
            thread = threading.Thread(target=read)
            thread.start()
            self.assertTrue(entered.wait(3))
            client.close()
            thread.join(3)
            self.assertFalse(thread.is_alive())
            self.assertEqual(len(errors), 1)
            client.close()

    def test_real_small_window_download_preserves_full_content(self):
        size = 4 * 1024 * 1024
        with loopback() as (client, server), patch.object(probe, 'DOWNLOAD_BYTES', size):
            errors = []
            def serve():
                try:
                    request = b''
                    while b'\r\n\r\n' not in request:
                        request += server.recv(4096)
                    server.sendall(b'HTTP/1.1 200 OK\r\nContent-Length: ' + str(size).encode() + b'\r\n\r\n')
                    for _ in range(size // 65536):
                        server.sendall(b'X' * 65536)
                except Exception as error:
                    errors.append(error)
                finally:
                    server.shutdown(socket.SHUT_WR)
            thread = threading.Thread(target=serve)
            thread.start()
            try:
                client.start_download('/fixture')
                result = client.finish(truncated=False)
                self.assertEqual(result['bytes'], size)
                self.assertEqual(result['sha256'], hashlib.sha256(b'X' * size).hexdigest())
            finally:
                client.close()
                thread.join(5)
            self.assertFalse(thread.is_alive())
            self.assertEqual(errors, [])

    def test_incomplete_kernel_abi_is_rejected(self):
        with self.assertRaises(ValueError):
            probe.received_bytes(SimpleNamespace(getsockopt=lambda *_args: b'\0' * 100))


class SamplingTests(unittest.TestCase):
    def test_all_coarse_poll_offsets_enter_window_before_expiry(self):
        # Fake elapsed time includes observer/health work. Previously a coarse
        # sample at e.g. 12980 slept 30s and missed the required final 5s entirely.
        for offset in range(30):
            for work in (0.1, 0.5, 1.5):
                with self.subTest(offset=offset, work=work):
                    now, last_owned, samples = 12950 + offset, None, []
                    while now < 13000:
                        samples.append(now)
                        last_owned = now
                        now += work
                        delay = probe.polling_delay(now, 0, expired=False)
                        self.assertGreater(delay, 0)
                        self.assertLessEqual(delay, 30)
                        now += delay
                    self.assertGreaterEqual(last_owned, 12995)
                    self.assertLessEqual(now - last_owned, 5)
                    self.assertLess(len(samples), 20)

    def test_uses_write_connection_origin_and_returns_to_normal_after_expiry(self):
        self.assertEqual(probe.polling_delay(13050, 100, expired=False), 30)
        self.assertEqual(probe.polling_delay(13085, 100, expired=False), 5)
        self.assertEqual(probe.polling_delay(13090, 100, expired=False), 1)
        self.assertEqual(probe.polling_delay(13101, 100, expired=True), 30)


class CleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_partial_admission_failure_always_cleans_exact_handles(self):
        handles, cleaned = [], []
        async def start(_target, marker, tracked):
            item = dict(marker=marker)
            tracked.append(item)
            handles.append(item)
            if len(handles) == 2:
                raise RuntimeError('admitted second command but reply failed')
        async def cleanup(tracked):
            cleaned.extend(tracked)
        with self.assertRaises(RuntimeError):
            await probe.admission(None, None, '/tmp/own', 'http://127.0.0.1:1', 4,
                                  SimpleNamespace(start_input=start, cleanup_handles=cleanup))
        self.assertEqual(cleaned, handles)
        self.assertEqual(len(cleaned), 2)


if __name__ == '__main__':
    unittest.main()
