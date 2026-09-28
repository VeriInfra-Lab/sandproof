"""SDK/gateway composition tests, launched only by the Rust test harness.

Management replies and envd are synthetic. All SDK data traffic goes through
the actual Rust gateway body/HTTP code on an ephemeral loopback listener.
"""

import importlib.metadata
from io import BytesIO
import json
from pathlib import Path
import sys
import threading
import unittest
from wsgiref.simple_server import make_server

import httpx
from e2b import Sandbox
from e2b.exceptions import AuthenticationException, SandboxException, TimeoutException

from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import EnvdWireContract, QuietHandler, SyntheticEnvd

# Exercise the same error classifier used by the opt-in real-VM script.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "system"))
from gateway_stream_limits import stream_failure


MIB = 1024 * 1024


class GatewayWireContract(EnvdWireContract):
    # Reuse the original command/file calls and their assertions through Rust.
    # The harness injects only two local fixture endpoints, never cloud URLs.
    server = None
    gateway_url = None

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
        for size in (8 * MIB - 1, 8 * MIB, 8 * MIB + 1):
            with self.subTest(size=size):
                before = len(self.app.records)
                body = BytesIO(b"X" * size)  # octet stream has no multipart overhead
                if size <= 8 * MIB:
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
            for size in (32 * MIB - 1, 32 * MIB, 32 * MIB + 1):
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

                    if size <= 32 * MIB:
                        download()
                        self.assertEqual(received, size)
                    else:
                        with self.assertRaises(httpx.RemoteProtocolError):
                            download()
                        self.assertGreater(received, 0)
                        self.assertLessEqual(received, 32 * MIB)
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


def main():
    manifest = json.loads(Path(__file__).with_name("contract.json").read_text())
    cases = unittest.defaultTestLoader.getTestCaseNames(GatewayWireContract)
    assert sorted(manifest["gateway_cases"]) == cases
    with isolated_sdk_environment():
        server = make_server("127.0.0.1", 0, SyntheticEnvd(), handler_class=QuietHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            print(f"127.0.0.1:{server.server_port}", flush=True)
            gateway_url = sys.stdin.readline().strip()
            assert gateway_url.startswith("http://127.0.0.1:")
            GatewayWireContract.server = server
            GatewayWireContract.gateway_url = gateway_url
            suite = unittest.defaultTestLoader.loadTestsFromTestCase(GatewayWireContract)
            result = unittest.TextTestRunner(verbosity=2).run(suite)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            assert not thread.is_alive(), "synthetic envd thread did not stop"
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
