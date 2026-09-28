"""SDK/gateway composition tests, launched only by the Rust test harness.

Management replies and envd are synthetic. All SDK data traffic goes through
the actual Rust gateway body/HTTP code on an ephemeral loopback listener.
"""

import importlib.metadata
from contextlib import ExitStack
from http.client import HTTPResponse
from io import BytesIO
import json
from pathlib import Path
import sys
import socket
from socketserver import ThreadingMixIn
import threading
import unittest
from urllib.parse import urlsplit
from wsgiref.simple_server import WSGIServer, make_server

import httpx
from e2b import Sandbox
from e2b.exceptions import AuthenticationException, RateLimitException, SandboxException, TimeoutException

from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import EnvdWireContract, QuietHandler, SyntheticEnvd

# Exercise the same error classifier used by the opt-in real-VM script.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "system"))
from gateway_stream_limits import UnknownLengthFile, stream_failure


MIB = 1024 * 1024


class GatewayWireContract(EnvdWireContract):
    # Reuse the original command/file calls and their assertions through Rust.
    # The harness injects only two local fixture endpoints, never cloud URLs.
    server = None
    gateway_url = None
    upload_bytes = 8 * MIB
    download_bytes = 32 * MIB

    def setUp(self):
        self.assertEqual(importlib.metadata.version("e2b"), "2.51.0")
        self.app = SyntheticEnvd()
        self.server.set_app(self.app)
        self.sandbox = Sandbox.connect(
            "probe-sandbox", api_url=f"http://127.0.0.1:{self.server.server_port}",
            sandbox_url=self.gateway_url, api_key="synthetic-local-only",
            domain="example.invalid", debug=False, request_timeout=15, retries=0,
        )

    def test_upload_boundary_rejection_never_reaches_envd(self):
        for size in (0, self.upload_bytes - 1, self.upload_bytes, self.upload_bytes + 1):
            with self.subTest(size=size):
                before = len(self.app.records)
                body = BytesIO(b"X" * size)  # octet stream has no multipart overhead
                if size <= self.upload_bytes:
                    self.sandbox.files.write("/tmp/input.txt", body)
                    self.assertEqual(self.app.records[-1]["body"], b"X" * size)
                else:
                    with self.assertRaisesRegex(SandboxException, "^413:"):
                        self.sandbox.files.write("/tmp/input.txt", body)
                    self.assertEqual(len(self.app.records), before)
        self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_download_limit_fixed_and_streamed_through_sdk(self):
        for known_length in (False, True):
            self.app.download_length = known_length
            for size in (0, self.download_bytes - 1, self.download_bytes, self.download_bytes + 1):
                with self.subTest(size=size, known_length=known_length):
                    self.app.download_size = size
                    before = len(self.app.records)
                    received = 0

                    def download():
                        nonlocal received
                        with self.sandbox.files.read("/tmp/input.txt", format="stream") as chunks:
                            for chunk in chunks:
                                self.assertEqual(chunk, b"X" * len(chunk))
                                received += len(chunk)

                    if size <= self.download_bytes:
                        download()
                        self.assertEqual(received, size)
                    else:
                        with self.assertRaises(httpx.RemoteProtocolError):
                            download()
                        self.assertGreater(received, 0)
                        self.assertLessEqual(received, self.download_bytes)
                    downloads = [r for r in self.app.records[before:] if r["path"] == "/files"]
                    self.assertEqual(len(downloads), 1)
                    self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_command_output_limit_fails_without_replaying_start(self):
        self.app.process.stdout_chunks = [b"X" * (64 * 1024)] * 512
        handle = self.sandbox.commands.run("synthetic-large-output", background=True)
        self.assertEqual(handle.pid, 4321)
        received = 0

        def output(chunk):
            nonlocal received
            self.assertEqual(chunk, "X" * len(chunk))
            received += len(chunk)

        with self.assertRaises(TimeoutException) as error:
            handle.wait(on_stdout=output)
        # The pinned SDK's exception name is misleading: its typed cause is
        # chunked HTTP EOF, not a deadline. The script must check that cause.
        self.assertTrue(stream_failure(error.exception))
        self.assertGreater(received, 0)
        self.assertLess(received, 32 * MIB)
        self.assertEqual(len(self.app.process.starts), 1)
        self.app.process.stdout_chunks = [b"synthetic\n"]
        self.assertEqual(self.sandbox.commands.run("printf healthy").stdout, "synthetic\n")
        self.assertEqual(len(self.app.process.starts), 2)

    def test_stream_classifier_rejects_unrelated_errors_and_deadlines(self):
        for error in (TimeoutException("deadline exceeded"), TimeoutError("deadline"),
                      SandboxException("unrelated RPC failure"), AuthenticationException("denied"),
                      AssertionError("test bug")):
            with self.subTest(error=type(error).__name__):
                self.assertFalse(stream_failure(error))

    def test_management_api_key_is_removed_from_data_request(self):
        with httpx.Client(base_url=self.gateway_url, trust_env=False, timeout=5) as client:
            response = client.get("/files?path=/tmp/input.txt", headers={
                "x-api-key": "synthetic-management-secret",
                "x-access-token": "synthetic-token",
            })
        self.assertEqual(response.status_code, 200)
        self.assertIsNone(self.app.records[-1]["api_key"])
        self.assertEqual(self.app.records[-1]["access_token"], "synthetic-token")


class GatewayStreamingContract(GatewayWireContract):
    def test_transfer_saturation_preserves_health_and_releases_capacity(self):
        gate = threading.Event()
        self.app.download_size = 128 * 1024
        self.app.download_gate = gate
        with ExitStack() as readers:
            streams = []
            try:
                for _ in range(3):  # fixture per-VM cap 4, with 1 reserved control slot
                    chunks = readers.enter_context(self.sandbox.files.read("/tmp/input.txt", format="stream"))
                    first = next(chunks)
                    self.assertGreater(len(first), 0)
                    streams.append((chunks, len(first)))
                before = len(self.app.records)
                with self.assertRaises(RateLimitException):
                    self.sandbox.files.read("/tmp/input.txt")
                self.assertEqual(len(self.app.records), before)
                with httpx.Client(base_url=self.gateway_url, trust_env=False, timeout=5) as client:
                    self.assertEqual(client.get("/health").status_code, 204)
            finally:
                gate.set()
            for chunks, size in streams:
                self.assertEqual(size + sum(len(chunk) for chunk in chunks), 128 * 1024)
        self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_idle_download_ends_as_error_and_next_request_works(self):
        gate = threading.Event()
        self.app.download_size = 128 * 1024
        self.app.download_gate = gate
        try:
            with self.sandbox.files.read("/tmp/input.txt", format="stream") as chunks:
                self.assertGreater(len(next(chunks)), 0)
                with self.assertRaises(httpx.RemoteProtocolError):
                    list(chunks)
        finally:
            gate.set()
        self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_idle_upload_is_rejected_and_next_request_works(self):
        address = urlsplit(self.gateway_url)
        with socket.create_connection((address.hostname, address.port), timeout=15) as client:
            client.sendall(b"POST /files?path=/tmp/input.txt HTTP/1.1\r\n"
                           b"Host: localhost\r\nContent-Type: application/octet-stream\r\n"
                           b"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nX\r\n")
            response = HTTPResponse(client)
            response.begin()
            self.assertEqual(response.status, 408)
            response.read()
        self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_unknown_length_upload_cannot_bypass_total_budget(self):
        for size in (0, self.upload_bytes - 1, self.upload_bytes, self.upload_bytes + 1):
            with self.subTest(size=size):
                stream = UnknownLengthFile(size)
                if size <= self.upload_bytes:
                    self.sandbox.files.write("/tmp/input.txt", stream)
                    self.assertEqual(len(self.app.records[-1]["body"]), size)
                    self.assertEqual(self.app.records[-1]["body"], b"X" * size)
                else:
                    with self.assertRaisesRegex(SandboxException, "^413:"):
                        self.sandbox.files.write("/tmp/input.txt", stream)
                self.assertEqual(self.sandbox.commands.run("printf healthy").exit_code, 0)

    def test_compressed_upload_rejected_and_download_negotiates_identity(self):
        before = len(self.app.records)
        with self.assertRaisesRegex(SandboxException, "^415:"):
            self.sandbox.files.write("/tmp/input.txt", BytesIO(b"must-not-be-written"), gzip=True)
        self.assertEqual(len(self.app.records), before)
        self.assertEqual(self.sandbox.files.read("/tmp/input.txt", gzip=True), "synthetic file content\n")
        self.assertEqual(self.app.records[-1]["accept_encoding"], "identity")

    def test_unexpected_compressed_upstream_is_not_forwarded(self):
        self.app.download_size = 10
        self.app.download_encoding = "gzip"
        with httpx.Client(base_url=self.gateway_url, trust_env=False, timeout=5) as client:
            response = client.get("/files?path=/tmp/input.txt")
        self.assertEqual(response.status_code, 502)
        self.assertEqual(response.content, b"")


class ThreadedEnvd(ThreadingMixIn, WSGIServer):
    # server_close joins every handler. A stuck fault barrier fails the Rust
    # harness timeout rather than being silently abandoned as a daemon thread.
    daemon_threads = False


def main():
    manifest = json.loads(Path(__file__).with_name("contract.json").read_text())
    with isolated_sdk_environment():
        server = make_server("127.0.0.1", 0, SyntheticEnvd(), server_class=ThreadedEnvd, handler_class=QuietHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            print(f"127.0.0.1:{server.server_port}", flush=True)
            settings = json.loads(sys.stdin.readline())
            gateway_url = settings["gateway_url"]
            assert gateway_url.startswith("http://127.0.0.1:")
            test_class = GatewayStreamingContract if settings["streaming"] else GatewayWireContract
            manifest_key = "gateway_streaming_cases" if settings["streaming"] else "gateway_cases"
            assert sorted(manifest[manifest_key]) == unittest.defaultTestLoader.getTestCaseNames(test_class)
            test_class.server = server
            test_class.gateway_url = gateway_url
            test_class.upload_bytes = settings["upload_bytes"]
            test_class.download_bytes = settings["download_bytes"]
            suite = unittest.defaultTestLoader.loadTestsFromTestCase(test_class)
            result = unittest.TextTestRunner(verbosity=2).run(suite)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            assert not thread.is_alive(), "synthetic envd thread did not stop"
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
