"""Pinned asynchronous SDK wire calls; synthetic envd does not execute commands."""

import base64
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
from sdk_data_samples import (
    COMMAND, ENV, FILES, NONZERO_COMMAND, NONZERO_RESULT, command_result,
    file_entries, validate_command, validate_content, validate_entries,
    validate_file_info, validate_nonzero,
)
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, ThreadedEnvd, configure_data_files


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
        for user in ("root", "ubuntu"):
            with self.subTest(user=user):
                directory = "/tmp/project/" + user
                stdout, stderr, self.app.process.exit_code = command_result(directory, user)
                self.app.process.stdout_chunks = [stdout.encode()]
                self.app.process.stderr_chunks = [stderr.encode()]
                result = await self.sandbox.commands.run(COMMAND, cwd=directory, envs=ENV, user=user)
                validate_command(result, directory, user)
                config = self.app.process.starts[-1].process
                self.assertEqual((config.cmd, list(config.args)),
                                 ("/bin/bash", ["-l", "-c", COMMAND]))
                self.assertEqual(config.cwd, directory)
                self.assertEqual(dict(config.envs), ENV)
                self.assertEqual(self.app.records[-1]["authorization"],
                                 "Basic " + base64.b64encode((user + ":").encode()).decode())
                self.assertEqual(self.app.records[-1]["access_token"], "synthetic-token")
                self.app.process.exit_code, stderr = NONZERO_RESULT
                self.app.process.stderr_chunks = [stderr.encode()]
                with self.assertRaises(CommandExitException) as error:
                    await self.sandbox.commands.run(NONZERO_COMMAND, cwd=directory, user=user)
                validate_nonzero(error.exception)
        self.assertEqual(len(self.app.process.starts), 4)

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
        self.assertEqual(await self.sandbox.files.read("/tmp/input.txt"), "synthetic file content\n")
        for user in ("root", "ubuntu"):
            with self.subTest(user=user):
                directory = "/tmp/project/" + user
                configure_data_files(self.app, directory)
                self.assertTrue(await self.sandbox.files.make_dir(directory, user=user))
                self.assertEqual(self.app.filesystem.mkdir_requests[-1].path, directory)
                entries = file_entries(directory)
                written = await self.sandbox.files.write_files(entries, user=user)
                self.assertEqual([entry.path for entry in written], [e["path"] for e in entries])
                posts = self.app.records[-len(FILES):]
                # Requests may arrive in either order; returned entries retain
                # the original input order, as checked above.
                self.assertEqual({p["query"]["path"][0]: p["body"] for p in posts},
                                 {f"{directory}/{name}": data for name, data in FILES.items()})
                self.assertTrue(all(p["query"]["username"] == [user] for p in posts))
                validate_entries(await self.sandbox.files.list(directory, depth=4, user=user), directory)
                self.assertEqual(self.app.filesystem.list_requests[-1].depth, 4)
                self.assertEqual(self.app.filesystem.list_requests[-1].path, directory)
                for name, content in FILES.items():
                    path = f"{directory}/{name}"
                    info = await self.sandbox.files.get_info(path, user=user)
                    validate_file_info(info, path, content)
                    self.assertEqual(info.mode, 0o644)
                    self.assertEqual(self.app.filesystem.stat_requests[-1].path, path)
                    validate_content(await self.sandbox.files.read(path, format="bytes", user=user), content)
                    async with await self.sandbox.files.read(path, format="stream", user=user) as chunks:
                        validate_content(b"".join([chunk async for chunk in chunks]), content)

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
