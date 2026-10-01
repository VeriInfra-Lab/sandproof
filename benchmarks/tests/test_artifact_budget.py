"""Trial evidence boundaries with the pinned SDK reader; no VM or task execution."""
import argparse
import asyncio
import contextlib
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

import httpx
from e2b import FileType
from e2b.sandbox_async.filesystem.filesystem import AsyncFileStreamReader, Filesystem
from harbor.environments.e2b import E2BEnvironment
from harbor.models.task.task import Task
from harbor.models.trial.paths import TrialPaths

from benchmarks.artifact_budget import (
    ArtifactBudget, ArtifactBudgetExceeded, DEFAULT_ARTIFACT_LIMITS,
    ArtifactStream, add_artifact_arguments, artifact_limits_from_args, json_string_bytes,
)
from benchmarks.sdk_environment import SdkEnvironment
from benchmarks import run_local as runner


class Chunks(httpx.AsyncByteStream):
    def __init__(self, chunks, *, error=None, stalled=None):
        self.chunks, self.error, self.stalled = chunks, error, stalled
        self.closed = False
        self.delivered = 0
        self.release = asyncio.Event()

    async def __aiter__(self):
        for chunk in self.chunks:
            self.delivered += 1
            yield chunk
        if self.stalled is not None:
            self.stalled.set()
            await self.release.wait()
        if self.error:
            raise self.error

    async def aclose(self):
        self.closed = True
        self.release.set()


def make_environment(root, *, max_bytes=100, max_entries=20, max_depth=16, max_file_bytes=None):
    env = object.__new__(SdkEnvironment)
    env.artifact_budget = ArtifactBudget(dict(max_bytes=max_bytes, max_entries=max_entries,
        max_depth=max_depth, max_file_bytes=max_file_bytes or max_bytes))
    env.events = []
    env.trial_paths = TrialPaths(root)
    env._sandbox = SimpleNamespace(files=SimpleNamespace(
        read=AsyncMock(), list=AsyncMock(return_value=[]),
        get_info=AsyncMock(return_value=SimpleNamespace(type=FileType.FILE))))
    return env


def readers(env, *bodies):
    streams = [Chunks(body) if isinstance(body, list) else body for body in bodies]
    env._sandbox.files.read.side_effect = [
        AsyncFileStreamReader(httpx.Response(200, stream=stream)) for stream in streams]
    return streams


class ArtifactBudgetTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    async def test_two_files_exact_bound_and_one_byte_over(self):
        for second, success in ((b'cd', True), (b'cde', False)):
            with self.subTest(second=second), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                env = make_environment(root, max_bytes=4)
                streams = readers(env, [b'ab'], [second])
                await env.download_file('/first', root / 'first')
                if success:
                    await env.download_file('/second', root / 'second')
                    self.assertEqual((root / 'second').read_bytes(), second)
                    self.assertEqual(env.artifact_budget.bytes, 4)
                else:
                    with self.assertRaisesRegex(ArtifactBudgetExceeded, 'artifact_bytes'):
                        await env.download_file('/second', root / 'second')
                    self.assertFalse((root / 'second').exists())
                    self.assertFalse((root / 'second.partial').exists())
                    self.assertEqual(env.artifact_budget.bytes, 2)
                self.assertEqual((root / 'first').read_bytes(), b'ab')
                self.assertTrue(all(stream.closed for stream in streams))
                self.assertEqual(env._sandbox.files.read.call_args.kwargs, {'format': 'stream'})

    async def test_independent_per_file_cap_and_no_following_chunk(self):
        env = make_environment(self.root, max_bytes=100, max_file_bytes=3)
        stream, = readers(env, [b'abc', b'd', b'unread'])
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'artifact_file_bytes'):
            await env.download_file('/a', self.root / 'a')
        self.assertEqual(stream.delivered, 2)
        self.assertTrue(stream.closed)
        self.assertEqual(env.artifact_budget.bytes, 3)
        self.assertEqual(list(self.root.iterdir()), [])

    async def test_zero_byte_files_consume_entries(self):
        env = make_environment(self.root, max_entries=2)
        readers(env, [], [])
        for name in ('a', 'b'):
            await env.download_file('/' + name, self.root / name)
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'artifact_entries'):
            await env.download_file('/c', self.root / 'c')
        self.assertEqual(env._sandbox.files.read.await_count, 2)
        self.assertEqual(env.artifact_budget.bytes, 0)
        self.assertEqual(env.artifact_budget.entries, 2)

    async def test_directory_depth_and_duplicate_traversal(self):
        env = make_environment(self.root, max_depth=2)
        env._sandbox.files.get_info.return_value.type = FileType.DIR
        env._sandbox.files.list.side_effect = [
            [SimpleNamespace(path='/logs/a', type=FileType.DIR)],
            [SimpleNamespace(path='/logs/a/b', type=FileType.DIR)],
            [SimpleNamespace(path='/logs/a/b/c', type=FileType.DIR)],
        ]
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'artifact_depth'):
            await env.download_dir('/logs', self.root)
        self.assertEqual(env._sandbox.files.list.await_count, 3)
        env = make_environment(self.root)
        readers(env, [])
        env._sandbox.files.get_info.side_effect = lambda path: SimpleNamespace(
            type=FileType.DIR if path == '/logs' else FileType.FILE)
        env._sandbox.files.list.return_value = [SimpleNamespace(path='/logs/same', type=FileType.FILE)] * 2
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'duplicate_artifact'):
            await env.download_dir('/logs', self.root)
        env._sandbox.files.read.assert_awaited_once()

    async def test_cancel_closes_stream_keeps_old_file_and_unrelated_partial(self):
        env = make_environment(self.root)
        (self.root / 'a').write_bytes(b'previous')
        (self.root / 'other.partial').write_bytes(b'not ours')
        stalled = asyncio.Event()
        stream, = readers(env, Chunks([b'prefix'], stalled=stalled))
        task = asyncio.create_task(env.download_file('/a', self.root / 'a'))
        await asyncio.wait_for(stalled.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(stream.closed)
        self.assertFalse((self.root / 'a.partial').exists())
        self.assertEqual((self.root / 'a').read_bytes(), b'previous')
        self.assertEqual((self.root / 'other.partial').read_bytes(), b'not ours')
        self.assertEqual(env.artifact_budget.bytes, 6)
        self.assertIsNotNone(env.artifact_budget.failure)

    async def test_cancel_during_sdk_eof_close_finishes_same_close_before_propagation(self):
        for repeated in (False, True):
            with self.subTest(repeated=repeated):
                env = make_environment(self.root)
                entered, release = asyncio.Event(), asyncio.Event()
                class SlowClose(Chunks):
                    closes = 0
                    async def aclose(self):
                        self.closes += 1
                        entered.set()
                        await release.wait()
                        self.closed = True
                stream, = readers(env, SlowClose([b'body']))
                task = asyncio.create_task(env.download_file('/a', self.root / 'a'))
                await asyncio.wait_for(entered.wait(), 1)
                task.cancel()
                if repeated:
                    # A loop callback gives the task its first cancellation
                    # before scheduling the second, with no timing sleep.
                    asyncio.get_running_loop().call_soon(task.cancel)
                release.set()
                with self.assertRaises(asyncio.CancelledError):
                    await asyncio.wait_for(task, 1)
                self.assertTrue(stream.closed)
                self.assertEqual(stream.closes, 1)
                self.assertFalse((self.root / 'a').exists())
                self.assertFalse((self.root / 'a.partial').exists())
                self.assertIsNotNone(env.artifact_budget.failure)

    async def test_new_cancellation_during_budget_failure_cleanup_is_not_swallowed(self):
        env = make_environment(self.root, max_bytes=1)
        entered, release = asyncio.Event(), asyncio.Event()
        class SlowClose(Chunks):
            async def aclose(self):
                entered.set()
                await release.wait()
                self.closed = True
        stream, = readers(env, SlowClose([b'ab']))
        task = asyncio.create_task(env.download_file('/a', self.root / 'a'))
        await asyncio.wait_for(entered.wait(), 1)
        task.cancel()
        release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(task, 1)
        self.assertTrue(task.cancelled())
        self.assertTrue(stream.closed)
        self.assertEqual(env.artifact_budget.violation, 'artifact_file_bytes_exceeded')
        self.assertFalse((self.root / 'a.partial').exists())

    async def test_stalled_close_has_one_bounded_deadline_and_explicit_failure(self):
        budget = ArtifactBudget(DEFAULT_ARTIFACT_LIMITS)
        budget.note_failure('earlier_artifact_failure')
        entered, release = asyncio.Event(), asyncio.Event()
        class SlowClose(Chunks):
            closes = 0
            async def aclose(self):
                self.closes += 1
                entered.set()
                await release.wait()
                self.closed = True
        stream = SlowClose([])
        reader = AsyncFileStreamReader(httpx.Response(200, stream=stream))
        owner = ArtifactStream(reader, budget, timeout=0.01)
        try:
            with self.assertRaises(TimeoutError):
                await asyncio.wait_for(owner.close(), 1)
            self.assertTrue(entered.is_set())
            self.assertEqual(budget.failure, 'earlier_artifact_failure')
            self.assertEqual(budget.report()['stream_close_failure'], 'artifact_stream_close_unconfirmed')
            with self.assertRaises(TimeoutError):
                await owner.close()
            self.assertEqual(stream.closes, 1)
        finally:
            release.set()
            await asyncio.wait_for(owner.task, 1)
        self.assertTrue(stream.closed)
        self.assertFalse(budget.report()['complete'])

    async def test_cancel_and_read_error_survive_close_timeout(self):
        for original_error in (None, TimeoutError('original read deadline')):
            with self.subTest(original_error=original_error):
                env = make_environment(self.root)
                entered, release = asyncio.Event(), asyncio.Event()
                class SlowClose(Chunks):
                    async def aclose(self):
                        entered.set()
                        await release.wait()
                        self.closed = True
                stream, = readers(env, SlowClose([b'body'], error=original_error))
                owners = []
                def short_owner(*args):
                    owner = ArtifactStream(*args, timeout=0.01)
                    owners.append(owner)
                    return owner
                with patch('benchmarks.artifact_budget.ArtifactStream', side_effect=short_owner):
                    task = asyncio.create_task(env.download_file('/a', self.root / 'a'))
                    await asyncio.wait_for(entered.wait(), 1)
                    task.cancel()
                    try:
                        with self.assertRaises(asyncio.CancelledError):
                            await asyncio.wait_for(task, 1)
                        self.assertFalse(stream.closed)
                        self.assertEqual(env.artifact_budget.failure, 'artifact_stream_close_unconfirmed')
                        self.assertFalse((self.root / 'a.partial').exists())
                    finally:
                        release.set()
                        await asyncio.wait_for(owners[0].task, 1)
                self.assertTrue(stream.closed)

        budget = ArtifactBudget(DEFAULT_ARTIFACT_LIMITS)
        release = asyncio.Event()
        class ReaderFailure:
            def __aiter__(self):
                return self
            async def __anext__(self):
                raise TimeoutError('original read deadline')
            async def aclose(self):
                await release.wait()
        from benchmarks.artifact_budget import closing_artifact_stream
        owners = []
        def short_owner(*args):
            owner = ArtifactStream(*args, timeout=0.01)
            owners.append(owner)
            return owner
        with patch('benchmarks.artifact_budget.ArtifactStream', side_effect=short_owner):
            try:
                with self.assertRaisesRegex(TimeoutError, 'original read deadline'):
                    async with closing_artifact_stream(ReaderFailure(), budget) as stream:
                        async for _ in stream:
                            pass
                self.assertEqual(budget.failure, 'artifact_stream_close_unconfirmed')
            finally:
                release.set()
                await asyncio.wait_for(owners[0].task, 1)

    async def test_retry_and_repeated_target_never_refund_budget(self):
        env = make_environment(self.root, max_bytes=6)
        streams = readers(env, Chunks([b'abc'], error=OSError('interrupted')), [b'def'], [b'x'])
        with self.assertRaises(OSError):
            await env.download_file('/a', self.root / 'a')
        await env.download_file('/a', self.root / 'a')
        with self.assertRaises(ArtifactBudgetExceeded):
            await env.download_file('/a', self.root / 'a')
        self.assertEqual(env.artifact_budget.bytes, 6)
        self.assertEqual(env.artifact_budget.entries, 3)
        self.assertEqual((self.root / 'a').read_bytes(), b'def')
        self.assertTrue(all(stream.closed for stream in streams))
        self.assertFalse(env.artifact_budget.report()['complete'])

    async def test_existing_partial_and_host_symlinks_are_not_touched(self):
        outside = self.root / 'outside'
        outside.mkdir(mode=0o700)
        canary = outside / 'canary'
        canary.write_bytes(b'untouched')
        trial = self.root / 'trial'
        trial.mkdir(mode=0o700)
        (trial / 'link').symlink_to(outside, target_is_directory=True)
        (trial / 'file-link').symlink_to(canary)
        (trial / 'a.partial').write_bytes(b'prior partial')
        (trial / 'b.partial').symlink_to(canary)
        for target in (trial / 'link/canary', trial / 'file-link', trial / 'a', trial / 'b', canary):
            with self.subTest(target=target):
                env = make_environment(trial)
                with self.assertRaises((ArtifactBudgetExceeded, OSError)):
                    await env.download_file('/a', target)
                env._sandbox.files.read.assert_not_awaited()
        self.assertEqual(canary.read_bytes(), b'untouched')
        self.assertEqual((trial / 'a.partial').read_bytes(), b'prior partial')
        self.assertTrue((trial / 'b.partial').is_symlink())

    async def test_guest_symlink_and_filtered_download_are_explicit_failures(self):
        env = make_environment(self.root)
        env._sandbox.files.get_info.return_value.type = FileType.SYMLINK
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'symlink'):
            await env.download_file('/link', self.root / 'a')
        env._sandbox.files.read.assert_not_awaited()
        env = make_environment(self.root)
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'filtered_artifact'):
            await env.download_dir_filtered(source_dir='/logs', target_dir=self.root, include=['*'])
        env._sandbox.files.list.assert_not_awaited()

    async def test_private_root_required_even_for_empty_download_directory(self):
        self.root.chmod(0o755)
        env = make_environment(self.root)
        with self.assertRaisesRegex(ArtifactBudgetExceeded, 'not_private'):
            await env.download_dir('/logs', self.root)
        env._sandbox.files.list.assert_not_awaited()

    async def test_event_output_count_and_byte_limits_never_silently_truncate(self):
        command, stdout, stderr = 'command', '\x00\U0001f600', 'error'
        required = json_string_bytes(command) + 1024 + json_string_bytes(stdout) + json_string_bytes(stderr)
        for bound, success in ((required, True), (required - 1, False)):
            env = make_environment(self.root, max_bytes=bound)
            result = SimpleNamespace(return_code=0, stdout=stdout, stderr=stderr)
            with patch.object(E2BEnvironment, 'exec', AsyncMock(return_value=result)) as execute:
                if success:
                    self.assertIs(await env.exec(command), result)
                    self.assertEqual(env.events[0]['stdout'], stdout)
                    self.assertLessEqual(len(json.dumps(env.events, indent=2)), env.artifact_budget.bytes)
                else:
                    with self.assertRaises(ArtifactBudgetExceeded):
                        await env.exec(command)
                    self.assertNotIn('stdout', env.events[0])
                    self.assertEqual(env.events[0]['error_type'], 'ArtifactBudgetExceeded')
                    with self.assertRaises(ArtifactBudgetExceeded):
                        await env.exec('diagnostic')
                execute.assert_awaited_once()
        env = make_environment(self.root, max_bytes=10000, max_entries=1)
        with patch.object(E2BEnvironment, 'exec', AsyncMock(return_value=result)) as execute:
            await env.exec(command)
            with self.assertRaises(ArtifactBudgetExceeded):
                await env.exec(command)
            execute.assert_awaited_once()
        self.assertEqual(len(env.events), 1)

    async def test_command_and_diagnostic_copies_share_download_budget(self):
        env = make_environment(self.root, max_bytes=1038)
        result = SimpleNamespace(return_code=0, stdout='abc', stderr='')
        with patch.object(E2BEnvironment, 'exec', AsyncMock(return_value=result)):
            await env.exec('c')  # 1024 + 3 + 5 + 2 = 1034
        env.retain_text(self.root / 'packages.txt', 'abc')
        readers(env, [b'x'])
        await env.download_file('/last', self.root / 'last')
        self.assertEqual(env.artifact_budget.bytes, 1038)
        with self.assertRaises(ArtifactBudgetExceeded):
            env.retain_text(self.root / 'diagnostics.txt', 'y')
        self.assertFalse((self.root / 'diagnostics.txt').exists())

    async def test_real_fixed_sdk_filesystem_stream_path_owns_response(self):
        # Original Filesystem.read and AsyncFileStreamReader, with a synthetic
        # HTTP transport that cannot issue any external request.
        body = Chunks([b'ab', b'cd'])
        async def respond(request):
            self.assertEqual(str(request.url), 'http://127.0.0.1/files?path=%2Ffixture')
            return httpx.Response(200, stream=body)
        async with httpx.AsyncClient(base_url='http://127.0.0.1', transport=httpx.MockTransport(respond)) as client:
            fs = object.__new__(Filesystem)
            from packaging.version import Version
            from e2b.connection_config import ConnectionConfig
            fs._envd_version = Version('0.9.0')
            fs._connection_config = ConnectionConfig()
            fs._envd_api_streaming = client
            fs.get_info = AsyncMock(return_value=SimpleNamespace(type=FileType.FILE))
            env = make_environment(self.root, max_bytes=4)
            env._sandbox.files = fs
            await env.download_file('/fixture', self.root / 'fixture')
        self.assertTrue(body.closed)
        self.assertEqual((self.root / 'fixture').read_bytes(), b'abcd')


class ArtifactConfigurationTests(unittest.TestCase):
    def test_explicit_cli_defaults_and_invalid_limits(self):
        parser = argparse.ArgumentParser()
        add_artifact_arguments(parser)
        self.assertEqual(artifact_limits_from_args(parser.parse_args([])), DEFAULT_ARTIFACT_LIMITS)
        for change in ({'max_bytes': 0}, {'max_depth': 65}, {'max_entries': True}, {'max_file_bytes': 96 * 1024**2 + 1},
                       {'max_file_bytes': 10**12}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                ArtifactBudget({**DEFAULT_ARTIFACT_LIMITS, **change})


class ArtifactRunnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_original_oracle_swallowed_download_error_cannot_false_pass(self):
        # Real pinned OracleAgent and Verifier, synthetic command results only.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            taskdir = source / 'sample'
            (taskdir / 'solution').mkdir(parents=True)
            (taskdir / 'tests').mkdir()
            (taskdir / 'environment').mkdir()
            (taskdir / 'instruction.md').write_text('Synthetic task, never executed.')
            (taskdir / 'task.toml').write_text('version = "1.0"\n')
            (taskdir / 'solution/solve.sh').write_text('# fixture only\n')
            (taskdir / 'tests/test.sh').write_text('# fixture only\n')
            output = root / 'output'
            output.mkdir(mode=0o700)
            task = Task(taskdir)
            env = make_environment(output / 'sample', max_bytes=100000)
            env.task_env_config = task.config.environment
            env.upload_dir = AsyncMock()
            env.scoped_exec_env = lambda _: contextlib.nullcontext()
            env.start = AsyncMock()
            env.sandbox_id, env.create_dispatched, env.kill_confirmed = 'fixture', True, False
            env._sandbox.kill = AsyncMock(return_value=True)
            env._sandbox.files.read.side_effect = [OSError('oracle log unavailable'),
                AsyncFileStreamReader(httpx.Response(200, stream=Chunks([b'1\n'])))]
            env._sandbox.files.get_info.side_effect = lambda path: SimpleNamespace(
                type=FileType.DIR if path == '/logs/verifier' else FileType.FILE)
            env._sandbox.files.list.return_value = [SimpleNamespace(path='/logs/verifier/reward.txt', type=FileType.FILE)]
            command = AsyncMock(return_value=SimpleNamespace(stdout='', stderr='', return_code=0))
            with (patch.object(E2BEnvironment, 'exec', command),
                  patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'fixture-only'}),
                  contextlib.redirect_stdout(io.StringIO())):
                row = await runner.run_task(SimpleNamespace(source=source, output=output, api_url='http://127.0.0.1'),
                    {'name': 'sample'}, {}, environment_factory=lambda **_: env)
            self.assertEqual(row['oracle'], 'passed')
            self.assertEqual(row['verifier'], 'passed')
            self.assertEqual(row['rewards'], {'reward': 1.0})
            self.assertEqual(row['status'], 'failed')
            self.assertEqual(row['reason'], 'artifact_evidence_incomplete')
            self.assertFalse(row['evidence_complete'])
            self.assertTrue(row['kill_confirmed'])
            self.assertEqual(json.loads((output / 'sample/result.json').read_text()), row)

    async def test_budget_violation_blocks_diagnostics_but_kill_runs_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            task = SimpleNamespace(name='sample', instruction='fixture',
                paths=SimpleNamespace(task_dir=root, environment_dir=root),
                config=SimpleNamespace(environment=SimpleNamespace(build_timeout_sec=1),
                    agent=SimpleNamespace(timeout_sec=1), verifier=SimpleNamespace(timeout_sec=1)))
            env = make_environment(root / 'sample', max_bytes=1)
            env.start = AsyncMock()
            env.sandbox_id, env.create_dispatched, env.kill_confirmed = 'fixture', True, False
            kill = env._sandbox.kill = AsyncMock(return_value=True)
            async def over_limit(*_):
                try:
                    env.artifact_budget.charge(2)
                except ArtifactBudgetExceeded:
                    pass  # Match upstream Oracle's swallowed download error.
            oracle = SimpleNamespace(run=AsyncMock(side_effect=over_limit))
            verifier = SimpleNamespace(verify=AsyncMock(return_value=SimpleNamespace(rewards={'reward': 1})))
            with (patch.object(runner, 'Task', return_value=task),
                  patch.object(runner, 'OracleAgent', return_value=oracle),
                  patch.object(runner, 'Verifier', return_value=verifier),
                  patch.object(E2BEnvironment, 'exec', AsyncMock()) as execute,
                  patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'fixture'}),
                  contextlib.redirect_stdout(io.StringIO())):
                row = await runner.run_task(SimpleNamespace(source=root, output=root, api_url='http://127.0.0.1'),
                    {'name': 'sample'}, {}, environment_factory=lambda **_: env)
            self.assertEqual(row['status'], 'failed')
            self.assertEqual(row['rewards'], {'reward': 1})
            self.assertEqual(row['verifier'], 'passed')
            self.assertFalse(row['evidence_complete'])
            self.assertEqual(set(row['diagnostic_errors'].values()), {'ArtifactBudgetExceeded'})
            execute.assert_not_awaited()
            kill.assert_awaited_once()
            self.assertTrue(row['kill_confirmed'])


if __name__ == '__main__':
    unittest.main()
