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
import threading
import unittest
from urllib.parse import urlsplit
from wsgiref.simple_server import make_server

import httpx
from pyqwest import WriteError as TransportWriteError
from e2b import Sandbox
from e2b.exceptions import AuthenticationException, RateLimitException, SandboxException, TimeoutException

from sdk_support import isolated_sdk_environment
from test_sdk_envd_async import AsyncEnvdWireContract
from test_sdk_envd_wire import EnvdWireContract, QuietHandler, SyntheticEnvd, ThreadedEnvd

# Exercise the same error classifier used by the opt-in real-VM script.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "system"))
from gateway_stream_limits import UnknownLengthFile, stream_failure
from gateway_async_stream_limits import ASYNC_CHUNK_EOF, ASYNC_BODY_EOF, async_stream_failure


MIB = 1024 * 1024


class GatewayWireContract(EnvdWireContract):
    # Reuse the original command/file calls and their assertions through Rust.
    # The harness injects only two local fixture endpoints, never cloud URLs.
    server = None
    gateway_url = None
    upload_bytes = 8 * MIB
    download_bytes = 32 * MIB

    def check_early_upload_rejection(self, _size):
        # Legacy uploads are buffered up to the cap before rejection.
        pass

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
                    self.check_early_upload_rejection(size)
                    try:
                        self.sandbox.files.write("/tmp/input.txt", body)
                    except SandboxException as error:
                        self.assertRegex(str(error), "^413:")
                    except httpx.WriteError as error:
                        # The pinned transport may see EPIPE before it reads
                        # that response. Neither outcome is upload success.
                        self.assertIs(type(error.__cause__), TransportWriteError)
                        self.assertTrue(str(error.__cause__).endswith("Broken pipe (os error 32)"))
                    else:
                        self.fail("oversized known upload returned success")
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

    def test_command_large_output_completes_without_replaying_start(self):
        self.app.process.stdout_chunks = [b"X" * (64 * 1024)] * 528
        handle = self.sandbox.commands.run("synthetic-large-output", background=True)
        self.assertEqual(handle.pid, 4321)
        received = 0

        def output(chunk):
            nonlocal received
            self.assertEqual(chunk, "X" * len(chunk))
            received += len(chunk)

        result = handle.wait(on_stdout=output)
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(received, 33 * MIB)
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
    def check_early_upload_rejection(self, size):
        # Observe the early 413 without racing a large client write against
        # the server's close. No body byte or envd request is needed.
        before = len(self.app.records)
        address = urlsplit(self.gateway_url)
        with socket.create_connection((address.hostname, address.port), timeout=15) as client:
            client.sendall((
                "POST /files?path=/tmp/input.txt HTTP/1.1\r\n"
                "Host: localhost\r\nContent-Type: application/octet-stream\r\n"
                f"Content-Length: {size}\r\nConnection: close\r\n\r\n"
            ).encode())
            response = HTTPResponse(client)
            response.begin()
            self.assertEqual(response.status, 413)
            response.read()
        self.assertEqual(len(self.app.records), before)

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


class GatewayAsyncContract(AsyncEnvdWireContract):
    server = None

    def setUp(self):
        # Environment and listener belong to the Rust composition harness.
        self.app = SyntheticEnvd()
        self.server.set_app(self.app)

    async def test_async_command_large_output_completes_without_replaying_start(self):
        self.app.process.stdout_chunks = [b"X" * (64 * 1024)] * 528
        received = 0

        def output(chunk):
            nonlocal received
            self.assertEqual(chunk, "X" * len(chunk))
            received += len(chunk)

        handle = await self.sandbox.commands.run(
            "synthetic-large-output", background=True, on_stdout=output)
        self.assertEqual(handle.pid, 4321)
        result = await handle.wait()
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(received, 33 * MIB)
        self.assertEqual(len(self.app.process.starts), 1)
        self.app.process.stdout_chunks = [b"synthetic\n"]
        self.assertEqual((await self.sandbox.commands.run("healthy")).stdout, "synthetic\n")
        self.assertEqual(len(self.app.process.starts), 2)

    def test_async_stream_classifier_rejects_unrelated_errors_and_deadlines(self):
        for message in (ASYNC_CHUNK_EOF, ASYNC_BODY_EOF):
            self.assertTrue(async_stream_failure(TimeoutException(message)))
            chained_deadline = TimeoutException(message)
            chained_deadline.__cause__ = TimeoutError("deadline")
            self.assertFalse(async_stream_failure(chained_deadline))
            self.assertFalse(async_stream_failure(TimeoutException(message + " extra")))
        for error in (TimeoutException("deadline exceeded"), TimeoutError("deadline"),
                      TimeoutException("unexpected EOF"), TimeoutException(ASYNC_CHUNK_EOF + " extra"),
                      SandboxException(ASYNC_CHUNK_EOF), AuthenticationException("denied"),
                      AssertionError("test bug")):
            with self.subTest(error=type(error).__name__):
                self.assertFalse(async_stream_failure(error))


class GatewayAsyncStreamingContract(GatewayAsyncContract):
    async def test_async_chunked_upload_budget_and_recovery(self):
        for size in (0, self.upload_bytes - 1, self.upload_bytes, self.upload_bytes + 1):
            with self.subTest(size=size):
                body = UnknownLengthFile(size)
                if size <= self.upload_bytes:
                    await self.sandbox.files.write("/tmp/input.txt", body)
                    self.assertEqual(self.app.records[-1]["body"], b"X" * size)
                else:
                    with self.assertRaisesRegex(SandboxException, "^413:"):
                        await self.sandbox.files.write("/tmp/input.txt", body)
                self.assertEqual((await self.sandbox.commands.run("healthy")).exit_code, 0)

    async def test_async_stream_download_budget_and_recovery(self):
        for known_length in (False, True):
            self.app.download_length = known_length
            for size in (0, self.download_bytes - 1, self.download_bytes, self.download_bytes + 1):
                with self.subTest(size=size, known_length=known_length):
                    self.app.download_size = size
                    received = 0
                    failed = False
                    try:
                        async with await self.sandbox.files.read("/tmp/input.txt", format="stream") as chunks:
                            async for chunk in chunks:
                                self.assertEqual(chunk, b"X" * len(chunk))
                                received += len(chunk)
                    except httpx.RemoteProtocolError:
                        if size <= self.download_bytes:
                            raise
                        failed = True
                    if size <= self.download_bytes:
                        self.assertEqual(received, size)
                    else:
                        self.assertTrue(failed, "oversized async download returned success")
                        self.assertGreater(received, 0)
                        self.assertLessEqual(received, self.download_bytes)
                    self.assertEqual((await self.sandbox.commands.run("healthy")).exit_code, 0)


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
            classes = [(GatewayStreamingContract, "gateway_streaming_cases"),
                       (GatewayAsyncStreamingContract, "gateway_async_streaming_cases")] if settings["streaming"] else [
                           (GatewayWireContract, "gateway_cases"), (GatewayAsyncContract, "gateway_async_cases")]
            suite = unittest.TestSuite()
            for test_class, manifest_key in classes:
                assert sorted(manifest[manifest_key]) == unittest.defaultTestLoader.getTestCaseNames(test_class)
                test_class.server = server
                test_class.gateway_url = gateway_url
                test_class.upload_bytes = settings["upload_bytes"]
                test_class.download_bytes = settings["download_bytes"]
                suite.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(test_class))
            result = unittest.TextTestRunner(verbosity=2).run(suite)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            assert not thread.is_alive(), "synthetic envd thread did not stop"
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
