"""Pinned asynchronous SDK wire calls; synthetic envd does not execute commands."""

import importlib.metadata
from io import BytesIO
import json
from pathlib import Path
import threading
import unittest
from wsgiref.simple_server import make_server

import httpx
from e2b import AsyncSandbox
from e2b.envd.process import process_pb
from e2b.exceptions import NotEnoughSpaceException
from e2b.sandbox.commands.command_handle import CommandExitException
from protobuf import Oneof

from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, ThreadedEnvd


class AsyncEnvdWireContract(unittest.IsolatedAsyncioTestCase):
    gateway_url = None

    def setUp(self):
        self.env_patch = isolated_sdk_environment()
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.app = SyntheticEnvd()
        self.server = make_server("127.0.0.1", 0, self.app,
                                  server_class=ThreadedEnvd, handler_class=QuietHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())

    async def asyncSetUp(self):
        self.assertEqual(importlib.metadata.version("e2b"), "2.51.0")
        url = f"http://127.0.0.1:{self.server.server_port}"
        self.sandbox = await AsyncSandbox.connect(
            "probe-sandbox", api_url=url, sandbox_url=self.gateway_url or url,
            api_key="synthetic-local-only", domain="example.invalid", debug=False,
            request_timeout=15, retries=0,
        )

    async def test_async_command_cwd_env_user_stdout_stderr_and_nonzero(self):
        self.app.process.stderr_chunks = [b"synthetic stderr\n"]
        result = await self.sandbox.commands.run(
            "printf synthetic", cwd="/tmp/project", envs={"SP_VALUE": "spaces and 中文"},
            user="ubuntu",
        )
        self.assertEqual((result.stdout, result.stderr, result.exit_code),
                         ("synthetic\n", "synthetic stderr\n", 0))
        config = self.app.process.starts[0].process
        self.assertEqual(config.cwd, "/tmp/project")
        self.assertEqual(dict(config.envs), {"SP_VALUE": "spaces and 中文"})
        self.assertEqual(self.app.records[-1]["authorization"], "Basic dWJ1bnR1Og==")
        self.assertEqual(self.app.records[-1]["access_token"], "synthetic-token")
        self.app.process.exit_code = 17
        with self.assertRaises(CommandExitException) as error:
            await self.sandbox.commands.run("exit 17")
        self.assertEqual(error.exception.exit_code, 17)
        self.assertEqual(len(self.app.process.starts), 2)

    async def test_async_background_reconnect_and_kill_do_not_replay_start(self):
        handle = await self.sandbox.commands.run("synthetic-background", background=True)
        self.assertEqual(handle.pid, 4321)
        await handle.disconnect()
        resumed = await self.sandbox.commands.connect(handle.pid)
        self.assertEqual(resumed.pid, handle.pid)
        self.assertEqual((await resumed.wait()).stdout, "synthetic\n")
        self.assertTrue(await resumed.kill())
        self.assertEqual(len(self.app.process.starts), 1)
        self.assertEqual(self.app.process.connects[0].process.selector, Oneof("pid", handle.pid))
        self.assertEqual(self.app.process.signals[0].process.selector, Oneof("pid", handle.pid))
        self.assertEqual(self.app.process.signals[0].signal, process_pb.Signal.SIGKILL)

    async def test_async_truncated_command_is_not_replayed_and_next_call_works(self):
        self.app.truncate_process_frame = True
        handle = await self.sandbox.commands.run("synthetic-side-effect", background=True)
        with self.assertRaisesRegex(Exception, "^Command ended without an end event$"):
            await handle.wait()
        self.assertEqual(len(self.app.process.starts), 1)
        self.app.truncate_process_frame = False
        self.assertEqual((await self.sandbox.commands.run("healthy")).stdout, "synthetic\n")
        self.assertEqual(len(self.app.process.starts), 2)

    async def test_async_file_info_batch_and_directory_requests(self):
        info = await self.sandbox.files.get_info("/tmp/input.txt", user="ubuntu")
        self.assertEqual((info.path, info.size, info.mode), ("/tmp/input.txt", 15, 0o644))
        self.assertEqual(self.app.records[-1]["authorization"], "Basic dWJ1bnR1Og==")
        self.assertTrue(await self.sandbox.files.make_dir("/tmp/project/nested", user="ubuntu"))
        entries = await self.sandbox.files.list("/tmp/project", depth=3, user="ubuntu")
        self.assertEqual(entries[0].path, "/tmp/input.txt")
        self.assertEqual(self.app.filesystem.list_requests[0].depth, 3)
        self.assertEqual(self.app.filesystem.mkdir_requests[0].path, "/tmp/project/nested")
        written = await self.sandbox.files.write_files([
            {"path": "/tmp/project/a.txt", "data": BytesIO(b"first")},
            {"path": "/tmp/project/nested/b.bin", "data": BytesIO(b"\x00\xffsecond")},
        ], user="ubuntu")
        self.assertEqual([entry.path for entry in written],
                         ["/tmp/project/a.txt", "/tmp/project/nested/b.bin"])
        posts = self.app.records[-2:]
        # Async uploads may reach the server in either order, even though the
        # SDK preserves input order in the returned result list.
        bodies_by_path = {p["query"]["path"][0]: p["body"] for p in posts}
        self.assertEqual(bodies_by_path, {
            "/tmp/project/a.txt": b"first",
            "/tmp/project/nested/b.bin": b"\x00\xffsecond",
        })
        self.assertTrue(all(p["query"]["username"] == ["ubuntu"] for p in posts))
        self.assertEqual(await self.sandbox.files.read("/tmp/input.txt"), "synthetic file content\n")

    async def test_async_batch_failure_does_not_replay_previous_uploads(self):
        # Async SDK uses gather: the other writes may already be in flight.
        # Wait for every request before reporting one failure, with no sleeps.
        self.app.file_status_by_path = {"/tmp/batch/second": 507}
        self.app.file_batch_barrier = threading.Barrier(3)
        before = len(self.app.records)
        with self.assertRaises(NotEnoughSpaceException):
            await self.sandbox.files.write_files([
                {"path": f"/tmp/batch/{name}", "data": BytesIO(name.encode())}
                for name in ("first", "second", "third")
            ])
        uploads = [r for r in self.app.records[before:] if r["path"] == "/files"]
        self.assertCountEqual([r["query"]["path"] for r in uploads],
                              [["/tmp/batch/first"], ["/tmp/batch/second"], ["/tmp/batch/third"]])
        self.assertEqual((await self.sandbox.commands.run("healthy")).exit_code, 0)

    async def test_async_truncated_download_fails_and_next_call_works(self):
        self.app.truncate_file = True
        with self.assertRaises(httpx.RemoteProtocolError):
            await self.sandbox.files.read("/tmp/input.txt")
        self.app.truncate_file = False
        self.assertEqual(await self.sandbox.files.read("/tmp/input.txt"), "synthetic file content\n")
        self.assertEqual(len([r for r in self.app.records if r["path"] == "/files"]), 2)


class AsyncEnvdManifest(unittest.TestCase):
    def test_manifest_matches_async_cases(self):
        manifest = json.loads(Path(__file__).with_name("contract.json").read_text())
        self.assertEqual(sorted(manifest["envd_async_cases"]),
                         unittest.defaultTestLoader.getTestCaseNames(AsyncEnvdWireContract))
