"""Original SDK command/file calls against a synthetic loopback envd.

The ConnectRPC application below uses the SDK's pinned generated schema, but
does not execute a process or touch a sandbox filesystem.
"""

import importlib.metadata
from io import BytesIO
import json
from socketserver import ThreadingMixIn
import threading
import unittest
from urllib.parse import parse_qs
from wsgiref.simple_server import ServerHandler, WSGIRequestHandler, WSGIServer, make_server

import httpx
from protobuf import Oneof

from e2b import Sandbox
from e2b.envd.filesystem import filesystem_connect, filesystem_pb
from e2b.envd.process import process_connect, process_pb
from e2b.exceptions import AuthenticationException, FileNotFoundException, NotEnoughSpaceException
from e2b.sandbox.commands.command_handle import CommandExitException

from sdk_support import SANDBOX, isolated_sdk_environment


class QuietHandler(WSGIRequestHandler):
    def log_message(self, *_args):
        pass

    def send_error(self, *args, **kwargs):
        try:
            super().send_error(*args, **kwargs)
        except (BrokenPipeError, ConnectionResetError):
            # An over-limit upload intentionally closes the upstream socket.
            # Other handler failures still reach the test server's error log.
            pass

    def handle(self):
        # pyqwest sends Connect streaming RPCs with Transfer-Encoding: chunked.
        # The standard-library WSGI test server does not decode that framing.
        self.raw_requestline = self.rfile.readline(65537)
        if len(self.raw_requestline) > 65536:
            self.requestline = ""
            self.request_version = ""
            self.command = ""
            self.send_error(414)
            return
        if not self.parse_request():
            return
        if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
            decoded = bytearray()
            while True:
                size_line = self.rfile.readline(128)
                if not size_line:
                    self.send_error(400)
                    return
                try:
                    size = int(size_line.split(b";", 1)[0], 16)
                except ValueError:
                    self.send_error(400)
                    return
                if size < 0:
                    self.send_error(400)
                    return
                if size == 0:
                    for _ in range(16):
                        trailer = self.rfile.readline(128)
                        if trailer in (b"\r\n", b"\n"):
                            break
                        if not trailer:
                            self.send_error(400)
                            return
                    else:
                        self.send_error(400)
                        return
                    break
                # This is a synthetic fixture cap, deliberately larger than
                # the gateway upload budgets under test. Process RPCs stay small.
                cap = 128 * 1024 * 1024 if self.path.startswith("/files?") else 1024 * 1024
                if len(decoded) + size > cap:
                    self.send_error(413)
                    return
                decoded.extend(self.rfile.read(size))
                if self.rfile.read(2) != b"\r\n":
                    self.send_error(400)
                    return
            self.rfile = BytesIO(decoded)
            del self.headers["Transfer-Encoding"]
            self.headers["Content-Length"] = str(len(decoded))
        handler = ServerHandler(
            self.rfile, self.wfile, self.get_stderr(), self.get_environ(),
            multithread=False,
        )
        handler.request_handler = self
        handler.run(self.server.get_app())


class ThreadedEnvd(ThreadingMixIn, WSGIServer):
    # Joining handlers makes a stuck barrier fail instead of hiding as a daemon.
    daemon_threads = False


class SyntheticProcess(process_connect.ProcessSync):
    def __init__(self):
        self.starts = []
        self.connects = []
        self.signals = []
        self.exit_code = 0
        self.omit_end = False
        self.stdout_chunks = [b"synthetic\n"]
        self.stderr_chunks = []

    def start(self, request, _context):
        self.starts.append(request)
        yield from self.events(process_pb.StartResponse)

    def connect(self, request, _context):
        self.connects.append(request)
        yield from self.events(process_pb.ConnectResponse)

    def send_signal(self, request, _context):
        self.signals.append(request)
        return process_pb.SendSignalResponse()

    def events(self, response):
        # Transport fixtures only: no shell execution or process-state model.
        yield response(
            event=process_pb.ProcessEvent(
                event=Oneof("start", process_pb.ProcessEvent.StartEvent(pid=4321))
            )
        )
        for channel, chunks in (("stdout", self.stdout_chunks), ("stderr", self.stderr_chunks)):
            for chunk in chunks:
                yield response(
                    event=process_pb.ProcessEvent(
                        event=Oneof(
                            "data",
                            process_pb.ProcessEvent.DataEvent(output=Oneof(channel, chunk)),
                        )
                    )
                )
        if not self.omit_end:
            yield response(
                event=process_pb.ProcessEvent(
                    event=Oneof(
                        "end",
                        process_pb.ProcessEvent.EndEvent(
                            exit_code=self.exit_code, exited=True
                        ),
                    )
                )
            )


class SyntheticFilesystem(filesystem_connect.FilesystemSync):
    def __init__(self):
        self.list_requests = []
        self.mkdir_requests = []
        self.stat_requests = []

    def stat(self, request, _context):
        self.stat_requests.append(request)
        return filesystem_pb.StatResponse(entry=filesystem_pb.EntryInfo(
            name="input.txt", path="/tmp/input.txt", type=filesystem_pb.FileType.FILE,
            size=15, mode=0o644,
        ))

    def list_dir(self, request, _context):
        self.list_requests.append(request)
        return filesystem_pb.ListDirResponse(entries=[
            filesystem_pb.EntryInfo(
                name="input.txt", path="/tmp/input.txt", type=filesystem_pb.FileType.FILE,
                size=15, mode=0o644,
            )
        ])

    def make_dir(self, request, _context):
        self.mkdir_requests.append(request)
        return filesystem_pb.MakeDirResponse()


class SyntheticEnvd:
    def __init__(self):
        self.records = []
        self.process = SyntheticProcess()
        self.process_app = process_connect.ProcessWSGIApplication(self.process)
        self.filesystem = SyntheticFilesystem()
        self.filesystem_app = filesystem_connect.FilesystemWSGIApplication(
            self.filesystem
        )
        self.file_status = 200
        self.file_statuses = []
        self.file_status_by_path = {}
        self.file_batch_barrier = None
        self.truncate_process_frame = False
        self.truncate_file = False
        self.download_size = None
        self.download_length = True
        self.download_encoding = None
        self.download_gate = None

    def __call__(self, environ, start_response):
        path = environ["PATH_INFO"]
        method = environ["REQUEST_METHOD"]
        size = int(environ.get("CONTENT_LENGTH") or 0)
        body = environ["wsgi.input"].read(size) if size else b""
        self.records.append(
            {
                "method": method,
                "path": path,
                "query": parse_qs(environ.get("QUERY_STRING", "")),
                "content_type": environ.get("CONTENT_TYPE", ""),
                "access_token": environ.get("HTTP_X_ACCESS_TOKEN"),
                "api_key": environ.get("HTTP_X_API_KEY"),
                "accept_encoding": environ.get("HTTP_ACCEPT_ENCODING"),
                "authorization": environ.get("HTTP_AUTHORIZATION"),
                "connect_timeout_ms": environ.get("HTTP_CONNECT_TIMEOUT_MS"),
                "body": body,
            }
        )
        if path.startswith("/process.Process/"):
            # The generated application consumes the request body itself.
            environ["wsgi.input"] = BytesIO(body)
            frames = self.process_app(environ, start_response)
            if not self.truncate_process_frame:
                return frames

            def cut_frame():
                try:
                    iterator = iter(frames)
                    yield next(iterator)  # complete Start event: the command exists
                    frame = next(iterator)
                    assert len(frame) > 5
                    yield frame[:-1]  # EOF inside the declared Connect data frame
                finally:
                    frames.close()

            return cut_frame()

        if path.startswith("/filesystem.Filesystem/"):
            environ["wsgi.input"] = BytesIO(body)
            return self.filesystem_app(environ, start_response)

        if path == "/v2/sandboxes/probe-sandbox/connect" and method == "POST":
            response = json.dumps(SANDBOX).encode()
            start_response(
                "200 OK", [("Content-Type", "application/json"),
                           ("Content-Length", str(len(response)))]
            )
            return [response]

        if path == "/files":
            status = self.file_statuses.pop(0) if self.file_statuses else self.file_status
            target = parse_qs(environ.get("QUERY_STRING", "")).get("path", [""])[0]
            status = self.file_status_by_path.get(target, status)
            if method == "POST" and self.file_batch_barrier is not None:
                self.file_batch_barrier.wait(timeout=10)
            if status != 200:
                response = b'{"message":"synthetic rejection"}'
                start_response(
                    f"{status} Synthetic Rejection",
                    [("Content-Type", "application/json"),
                     ("Content-Length", str(len(response)))],
                )
                return [response]
            if method == "GET":
                if self.download_size is not None:
                    headers = [("Content-Type", "application/octet-stream")]
                    if self.download_encoding:
                        headers.append(("Content-Encoding", self.download_encoding))
                    if self.download_length:
                        headers.append(("Content-Length", str(self.download_size)))
                    start_response("200 OK", headers)

                    def chunks():
                        remaining = self.download_size
                        gate = self.download_gate
                        while remaining:
                            size = min(remaining, 64 * 1024)
                            yield b"X" * size
                            remaining -= size
                            if gate is not None and remaining:
                                gate.wait()  # explicit fault barrier; harness bounds the test
                                gate = None

                    return chunks()
                response = b"synthetic file content\n"
                start_response(
                    "200 OK", [("Content-Type", "text/plain"),
                               ("Content-Length", str(len(response) + int(self.truncate_file)))]
                )
                return [response]
            if method == "POST":
                uploaded_path = parse_qs(environ.get("QUERY_STRING", "")).get("path", ["/tmp/input.txt"])[0]
                response = json.dumps(
                    [{"name": uploaded_path.rsplit("/", 1)[-1], "path": uploaded_path, "type": "file"}]
                ).encode()
                start_response(
                    "200 OK", [("Content-Type", "application/json"),
                               ("Content-Length", str(len(response)))]
                )
                return [response]

        if path == "/health":
            start_response("204 No Content", [])
            return []

        response = b'{"message":"unexpected synthetic request"}'
        start_response(
            "500 Unexpected Request",
            [("Content-Type", "application/json"), ("Content-Length", str(len(response)))],
        )
        return [response]


class EnvdWireContract(unittest.TestCase):
    def setUp(self):
        self.assertEqual(importlib.metadata.version("e2b"), "2.51.0")
        self.env_patch = isolated_sdk_environment()
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.app = SyntheticEnvd()
        self.server = make_server("127.0.0.1", 0, self.app, handler_class=QuietHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        url = f"http://127.0.0.1:{self.server.server_port}"
        self.sandbox = Sandbox.connect(
            "probe-sandbox", api_url=url, sandbox_url=url,
            api_key="synthetic-local-only", domain="example.invalid",
            debug=False, request_timeout=5, retries=0,
        )

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def test_command_uses_connect_stream_and_preserves_result(self):
        result = self.sandbox.commands.run("printf synthetic")
        self.assertEqual((result.stdout, result.stderr, result.exit_code),
                         ("synthetic\n", "", 0))
        request = self.app.process.starts[0]
        self.assertEqual(request.process.cmd, "/bin/bash")
        self.assertEqual(list(request.process.args), ["-l", "-c", "printf synthetic"])
        rpc = self.app.records[-1]
        self.assertEqual((rpc["method"], rpc["path"]),
                         ("POST", "/process.Process/Start"))
        self.assertEqual(rpc["access_token"], "synthetic-token")

    def test_background_command_returns_handle_then_result(self):
        handle = self.sandbox.commands.run("sleep 1", background=True)
        self.assertEqual(handle.pid, 4321)
        self.assertEqual(handle.wait().exit_code, 0)
        self.assertEqual(len(self.app.process.starts), 1)

    def test_command_cwd_env_user_and_stderr_are_preserved(self):
        self.app.process.stderr_chunks = [b"synthetic stderr\n"]
        result = self.sandbox.commands.run("printf synthetic", cwd="/tmp/project",
                                           envs={"SP_VALUE": "spaces and \u4e2d\u6587"}, user="ubuntu")
        self.assertEqual((result.stdout, result.stderr, result.exit_code),
                         ("synthetic\n", "synthetic stderr\n", 0))
        request = self.app.process.starts[0].process
        self.assertEqual(request.cwd, "/tmp/project")
        self.assertEqual(dict(request.envs), {"SP_VALUE": "spaces and \u4e2d\u6587"})
        self.assertEqual(self.app.records[-1]["authorization"], "Basic dWJ1bnR1Og==")

    def test_reconnect_and_signal_use_known_pid_without_start_replay(self):
        handle = self.sandbox.commands.run("synthetic-background", background=True)
        handle.disconnect()
        resumed = self.sandbox.commands.connect(handle.pid)
        self.assertEqual(resumed.pid, handle.pid)
        self.assertEqual(resumed.wait().stdout, "synthetic\n")
        self.assertTrue(resumed.kill())
        self.assertEqual(len(self.app.process.starts), 1)
        self.assertEqual(self.app.process.connects[0].process.selector, Oneof("pid", handle.pid))
        self.assertEqual(self.app.process.signals[0].process.selector, Oneof("pid", handle.pid))
        self.assertEqual(self.app.process.signals[0].signal, process_pb.Signal.SIGKILL)

    def test_file_info_and_batch_upload_preserve_paths_and_bytes(self):
        info = self.sandbox.files.get_info("/tmp/input.txt", user="ubuntu")
        self.assertEqual((info.path, info.size, info.mode), ("/tmp/input.txt", 15, 0o644))
        self.assertEqual(self.app.filesystem.stat_requests[0].path, "/tmp/input.txt")
        self.assertEqual(self.app.records[-1]["authorization"], "Basic dWJ1bnR1Og==")
        entries = [{"path": "/tmp/project/a.txt", "data": BytesIO(b"first")},
                   {"path": "/tmp/project/nested/b.bin", "data": BytesIO(b"\x00\xffsecond")}]
        written = self.sandbox.files.write_files(entries, user="ubuntu")
        self.assertEqual([entry.path for entry in written], [entry["path"] for entry in entries])
        posts = self.app.records[-2:]
        self.assertEqual([p["query"]["path"][0] for p in posts], [e["path"] for e in entries])
        self.assertEqual([p["body"] for p in posts], [b"first", b"\x00\xffsecond"])
        self.assertTrue(all(p["query"]["username"] == ["ubuntu"] for p in posts))

    def test_batch_failure_does_not_replay_previous_uploads(self):
        self.app.file_statuses = [200, 507]
        before = len(self.app.records)
        with self.assertRaises(NotEnoughSpaceException):
            self.sandbox.files.write_files([
                {"path": f"/tmp/batch/{name}", "data": BytesIO(name.encode())}
                for name in ("first", "second", "third")
            ])
        uploads = [r for r in self.app.records[before:] if r["path"] == "/files"]
        self.assertEqual([r["query"]["path"] for r in uploads],
                         [["/tmp/batch/first"], ["/tmp/batch/second"]])
        self.assertEqual(self.sandbox.commands.run("healthy").exit_code, 0)

    def test_nonzero_command_exit_is_not_success(self):
        self.app.process.exit_code = 17
        with self.assertRaises(CommandExitException) as failure:
            self.sandbox.commands.run("exit 17")
        self.assertEqual(failure.exception.exit_code, 17)
        self.assertEqual(len(self.app.process.starts), 1)

    def test_command_missing_end_is_not_auto_retried(self):
        self.app.process.omit_end = True
        with self.assertRaisesRegex(Exception, "without an end event"):
            self.sandbox.commands.run("touch /tmp/side-effect")
        self.assertEqual(len(self.app.process.starts), 1)

    def test_command_truncated_frame_is_not_replayed_and_next_command_works(self):
        self.app.truncate_process_frame = True
        handle = self.sandbox.commands.run("touch /tmp/side-effect", background=True)
        self.assertEqual(handle.pid, 4321)
        with self.assertRaisesRegex(Exception, "^Command ended without an end event$"):
            handle.wait()
        self.assertEqual(len(self.app.process.starts), 1)
        self.app.truncate_process_frame = False
        self.assertEqual(self.sandbox.commands.run("printf healthy").stdout, "synthetic\n")
        self.assertEqual(len(self.app.process.starts), 2)

    def test_file_truncated_download_is_not_returned_as_success(self):
        self.app.truncate_file = True
        with self.assertRaises(httpx.RemoteProtocolError):
            self.sandbox.files.read("/tmp/input.txt")
        downloads = [record for record in self.app.records
                     if record["path"] == "/files" and record["method"] == "GET"]
        self.assertEqual(len(downloads), 1)
        self.app.truncate_file = False
        self.assertEqual(self.sandbox.files.read("/tmp/input.txt"), "synthetic file content\n")

    def test_file_write_and_read_use_envd_files_and_access_token(self):
        written = self.sandbox.files.write("/tmp/input.txt", "synthetic input")
        content = self.sandbox.files.read("/tmp/input.txt")
        self.assertEqual(written.path, "/tmp/input.txt")
        self.assertEqual(content, "synthetic file content\n")
        post, get = self.app.records[-2:]
        self.assertEqual((post["method"], post["path"]), ("POST", "/files"))
        self.assertEqual((get["method"], get["path"]), ("GET", "/files"))
        self.assertEqual(post["query"]["path"], ["/tmp/input.txt"])
        self.assertEqual(get["query"]["path"], ["/tmp/input.txt"])
        self.assertTrue(post["content_type"].startswith("multipart/form-data;"))
        self.assertIn(b"synthetic input", post["body"])
        self.assertEqual([post["access_token"], get["access_token"]],
                         ["synthetic-token", "synthetic-token"])

    def test_file_rejections_are_distinct(self):
        self.app.file_status = 401
        with self.assertRaises(AuthenticationException):
            self.sandbox.files.read("/tmp/input.txt")
        self.app.file_status = 404
        with self.assertRaises(FileNotFoundException):
            self.sandbox.files.read("/tmp/input.txt")

    def test_filelike_upload_uses_octet_stream_by_default(self):
        self.sandbox.files.write("/tmp/input.txt", BytesIO(b"streamed synthetic"))
        request = self.app.records[-1]
        self.assertEqual(request["content_type"], "application/octet-stream")
        self.assertEqual(request["body"], b"streamed synthetic")
        self.assertEqual(request["query"]["path"], ["/tmp/input.txt"])

    def test_directory_calls_use_filesystem_rpc(self):
        self.assertTrue(self.sandbox.files.make_dir("/tmp/project"))
        entries = self.sandbox.files.list("/tmp", depth=2)
        self.assertEqual(self.app.filesystem.mkdir_requests[0].path, "/tmp/project")
        self.assertEqual(self.app.filesystem.list_requests[0].path, "/tmp")
        self.assertEqual(self.app.filesystem.list_requests[0].depth, 2)
        self.assertEqual(entries[0].path, "/tmp/input.txt")
        self.assertEqual([r["path"] for r in self.app.records[-2:]], [
            "/filesystem.Filesystem/MakeDir", "/filesystem.Filesystem/ListDir"
        ])

    def test_sdk_forwards_path_that_server_must_check(self):
        self.sandbox.files.read("/tmp/../etc/passwd")
        request = self.app.records[-1]
        self.assertEqual(request["query"]["path"], ["/tmp/../etc/passwd"])


class EnvdManifest(unittest.TestCase):
    def test_manifest_matches_executable_cases(self):
        from pathlib import Path

        manifest = json.loads(Path(__file__).with_name("contract.json").read_text())
        self.assertEqual(sorted(manifest["envd_cases"]),
                         unittest.defaultTestLoader.getTestCaseNames(EnvdWireContract))
        all_cases = set(manifest["tested_cases"] + manifest["envd_cases"] + manifest["envd_async_cases"])
        self.assertTrue(all(call["evidence_case"] in all_cases for call in manifest["wire_calls"]))
        self.assertEqual(len({call["sdk_call"] for call in manifest["wire_calls"]}),
                         len(manifest["wire_calls"]))
