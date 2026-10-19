"""Actual pinned SDK upload/cancellation against a private loopback receiver."""
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import tempfile
import threading
import time
import unittest

from e2b import Sandbox
from e2b.connection_config import ConnectionConfig
from packaging.version import Version
from sdk_support import isolated_sdk_environment
from tools.template_build_store import BuildError
from tools.template_build_upload import upload


class UploadWireTests(unittest.TestCase):
    def setUp(self):
        environment = isolated_sdk_environment()
        environment.start()
        self.addCleanup(environment.stop)
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.source = Path(temporary.name).resolve() / 'payload'
        self.data = bytes(range(256)) * 1024
        self.source.write_bytes(self.data)
        self.source.chmod(0o400)
        self.pin = {'bytes': len(self.data), 'sha256': hashlib.sha256(self.data).hexdigest()}
        self.received, self.errors, self.disconnected = [], [], threading.Event()
        self.mode, self.now = 'success', 0
        owner = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_args): pass

            def do_POST(self):
                try:
                    self.connection.settimeout(4)
                    result = bytearray()
                    if self.headers.get('Transfer-Encoding') == 'chunked':
                        while True:
                            size = int(self.rfile.readline(128).strip(), 16)
                            if not size:
                                assert self.rfile.readline(128) == b'\r\n'
                                break
                            assert size <= 65536 and len(result) + size <= len(owner.data)
                            result.extend(self.rfile.read(size))
                            assert self.rfile.read(2) == b'\r\n'
                    else:
                        size = int(self.headers['Content-Length'])
                        assert size <= len(owner.data)
                        result.extend(self.rfile.read(size))
                    owner.received.append((self.path, dict(self.headers), bytes(result)))
                    if owner.mode == 'stalled':
                        assert self.rfile.read(1) == b''
                        owner.disconnected.set()
                        return
                    if owner.mode == 'late': owner.now = 100
                    response = json.dumps([{'name': 'payload', 'path': '/input/payload', 'type': 'file'}]).encode()
                    self.send_response(200)
                    self.send_header('Content-Length', str(len(response)))
                    self.end_headers()
                    self.wfile.write(response)
                except BaseException as error:
                    owner.errors.append(type(error).__name__)
                finally:
                    self.close_connection = True

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': 0.05})
        self.thread.start()
        self.addCleanup(self.stop)
        url = f'http://127.0.0.1:{self.server.server_port}'
        self.version = Version('0.9.0')
        self.sandbox = Sandbox(sandbox_id='original-builder', sandbox_domain='example.invalid',
            envd_version=self.version, envd_access_token='synthetic-token',
            connection_config=ConnectionConfig(api_url=url, sandbox_url=url, domain='example.invalid',
                api_key='synthetic-key', retries=0, extra_sandbox_headers={
                    'X-Access-Token': 'synthetic-token', 'x-sandproof-build-owner': '7',
                    'x-sandproof-build-key': 'synthetic-helper-key'}))

    def stop(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())
        self.assertEqual(self.errors, [])

    def write(self, *, deadline=100, clock=None):
        upload(self.sandbox, self.version, '/input/payload', self.source,
               deadline, clock or (lambda: self.now), expected=self.pin)

    def test_streamed_bytes_and_original_private_identity_reach_only_files_endpoint(self):
        self.write()
        self.assertEqual(len(self.received), 1)
        path, headers, data = self.received[0]
        self.assertEqual(path, '/files?path=%2Finput%2Fpayload&username=root')
        self.assertEqual(data, self.data)
        lower = {key.lower(): value for key, value in headers.items()}
        self.assertEqual(lower['x-access-token'], 'synthetic-token')
        self.assertEqual(lower['x-sandproof-build-owner'], '7')
        self.assertEqual(lower['x-sandproof-build-key'], 'synthetic-helper-key')
        # This private center resolves the original VM from tenant + access
        # token; its route does not use the public E2b-Sandbox-Id header.
        self.assertEqual(self.sandbox.sandbox_id, 'original-builder')

    def test_stalled_response_is_cancelled_without_replay_and_connection_is_released(self):
        self.mode = 'stalled'
        with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'):
            self.write(deadline=time.monotonic() + 1, clock=time.monotonic)
        self.assertTrue(self.disconnected.wait(timeout=3))
        self.assertEqual(len(self.received), 1)

    def test_late_success_and_expired_start_cannot_authorize_next_phase(self):
        self.now = 100
        with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'): self.write()
        self.assertEqual(self.received, [])
        self.now, self.mode = 0, 'late'
        with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'): self.write()
        self.assertEqual(len(self.received), 1)
