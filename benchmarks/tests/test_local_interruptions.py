"""Regression cases for interrupted commands and best-effort diagnostics."""
import asyncio
from benchmarks.artifact_budget import ArtifactBudget, DEFAULT_ARTIFACT_LIMITS
import contextlib
import errno
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

from benchmarks import run_local as runner


class InterruptedRunTests(unittest.TestCase):
    def test_interrupted_command_records_attempt_without_replay(self):
        env = object.__new__(runner.LocalBaseEnvironment)
        env.artifact_budget = ArtifactBudget(DEFAULT_ARTIFACT_LIMITS)
        env.events = []
        call = AsyncMock(side_effect=TimeoutError("command deadline"))
        with patch.object(runner.E2BEnvironment, "exec", call):
            with self.assertRaises(TimeoutError):
                asyncio.run(env.exec("synthetic-command", timeout_sec=1))
        call.assert_awaited_once()
        self.assertEqual(len(env.events), 1)
        self.assertEqual(env.events[0]["command"], "synthetic-command")
        self.assertEqual(env.events[0]["error_type"], "TimeoutError")
        self.assertIsNone(env.events[0]["return_code"])

    def assert_failed_stage(self, stage, logs_available=False, completed_reward=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            task = SimpleNamespace(
                name="sample", instruction="synthetic fixture",
                paths=SimpleNamespace(environment_dir=root / "environment", task_dir=root / "task"),
                config=SimpleNamespace(
                    environment=SimpleNamespace(build_timeout_sec=1),
                    agent=SimpleNamespace(timeout_sec=1),
                    verifier=SimpleNamespace(timeout_sec=1)),
            )
            async def download(remote, target):
                if not logs_available:
                    raise OSError("logs unavailable")
                target.write_text("1\n" if remote.endswith("reward.txt") else "partial guest log\n")

            env = SimpleNamespace(
                sandbox_id="sb-fixture", create_dispatched=True, kill_confirmed=False,
                events=[], start=AsyncMock(side_effect=(
                    TimeoutError("initialization deadline") if stage == "initialization" else None)),
                download_file=AsyncMock(side_effect=download),
                exec=AsyncMock(side_effect=None if logs_available else OSError("observation unavailable"),
                               return_value=SimpleNamespace(stdout="guest observations\n", stderr="")),
            )

            env.retain_text = lambda path, content: path.write_text(content)

            async def stop():
                env.kill_confirmed = True

            env.stop = AsyncMock(side_effect=stop)
            oracle = SimpleNamespace(run=AsyncMock(side_effect=(
                TimeoutError("oracle deadline") if stage == "oracle" else None)))
            verifier = SimpleNamespace(verify=AsyncMock(
                side_effect=TimeoutError("verifier deadline") if completed_reward is None else None,
                return_value=SimpleNamespace(rewards={"reward": completed_reward})))
            args = SimpleNamespace(source=root, output=root, api_url="http://127.0.0.1:1")
            with (
                patch.object(runner, "Task", return_value=task),
                patch.object(runner, "LocalBaseEnvironment", return_value=env),
                patch.object(runner, "OracleAgent", return_value=oracle),
                patch.object(runner, "Verifier", return_value=verifier),
                patch.dict(os.environ, {"SANDPROOF_TEST_API_KEY": "fixture-key"}),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                row = asyncio.run(runner.run_task(args, {"name": "sample"}, {}))
            self.assertEqual(row["status"], "timeout" if completed_reward is None else "failed")
            self.assertEqual(row["stage"], stage)
            self.assertTrue(row["kill_confirmed"])
            self.assertEqual(row["sandbox_id"], "sb-fixture")
            if logs_available:
                self.assertNotIn("diagnostic_errors", row)
                if completed_reward is None:
                    self.assertEqual((root / "sample/verifier/reward.txt").read_text(), "1\n")
                    self.assertNotIn("rewards", row)
                else:
                    self.assertEqual(row["rewards"], {"reward": completed_reward})
                self.assertEqual((root / "sample/environment-diagnostics.txt").read_text(), "guest observations\n")
            else:
                self.assertEqual(set(row["diagnostic_errors"].values()), {"OSError"})
            env.start.assert_awaited_once()
            env.stop.assert_awaited_once()
            if stage == "initialization":
                oracle.run.assert_not_awaited()
                verifier.verify.assert_not_awaited()
                self.assertEqual(row["oracle"], "not_run")
                self.assertEqual(row["verifier"], "not_run")
            elif stage == "oracle":
                verifier.verify.assert_not_awaited()
                self.assertEqual(row["oracle"], "timeout")
                self.assertEqual(row["verifier"], "not_run")
            else:
                self.assertEqual(row["oracle"], "passed")
                self.assertEqual(row["verifier"], "timeout" if completed_reward is None else "failed")

    def test_oracle_timeout_still_cleans_up_when_log_downloads_fail(self):
        self.assert_failed_stage("oracle")

    def test_verifier_timeout_keeps_oracle_result_and_cleans_up(self):
        self.assert_failed_stage("verifier")

    def test_initialization_timeout_never_starts_oracle_or_verifier(self):
        self.assert_failed_stage("initialization")

    def test_late_reward_in_partial_logs_does_not_turn_timeout_into_pass(self):
        self.assert_failed_stage("verifier", logs_available=True)

    def test_failed_score_keeps_verifier_phase_after_inventory(self):
        self.assert_failed_stage("verifier", logs_available=True, completed_reward=0)


class LocalFinalizationTests(unittest.TestCase):
    @contextlib.contextmanager
    def fixture(self, *, write_failure=None, oracle_error=None, cleanup_error=None, task_name='sample'):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            task = SimpleNamespace(
                name=task_name, instruction="fixture",
                paths=SimpleNamespace(environment_dir=root / "environment", task_dir=root / "task"),
                config=SimpleNamespace(environment=SimpleNamespace(build_timeout_sec=1),
                                       agent=SimpleNamespace(timeout_sec=1),
                                       verifier=SimpleNamespace(timeout_sec=1)),
            )
            env = SimpleNamespace(
                sandbox_id="sb-fixture", create_dispatched=True, kill_confirmed=False,
                events=[], start=AsyncMock(), download_file=AsyncMock(side_effect=OSError("no log")),
                exec=AsyncMock(return_value=SimpleNamespace(stdout="", stderr="")),
            )

            env.retain_text = lambda path, content: path.write_text(content)

            async def stop():
                if cleanup_error is not None:
                    raise cleanup_error
                env.kill_confirmed = True

            env.stop = AsyncMock(side_effect=stop)
            oracle = SimpleNamespace(run=AsyncMock(side_effect=oracle_error))
            verifier = SimpleNamespace(verify=AsyncMock(return_value=SimpleNamespace(rewards={"reward": 1})))
            original_write = Path.write_text

            def write(path, *args, **kwargs):
                if path.name == write_failure:
                    raise OSError(errno.ENOSPC, "injected local report disk full")
                return original_write(path, *args, **kwargs)

            args = SimpleNamespace(source=root, output=root, api_url="http://127.0.0.1:1")
            with (
                patch.object(runner, "Task", return_value=task),
                patch.object(runner, "LocalBaseEnvironment", return_value=env),
                patch.object(runner, "OracleAgent", return_value=oracle),
                patch.object(runner, "Verifier", return_value=verifier),
                patch.object(Path, "write_text", write),
                patch.dict(os.environ, {"SANDPROOF_TEST_API_KEY": "fixture-key"}),
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                yield args, env, oracle, verifier, root / task_name, output

    def test_generic_browser_reward_requires_actual_execution_evidence(self):
        complete = ('Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n'
                    '  No alert detected\n')
        for stdout, passed in ((None, False), ('PASSED\n', False),
                               (complete + '  Failed to load URL: unavailable\n', False),
                               (complete, True)):
            with self.subTest(stdout=stdout), self.fixture(task_name='filter-js-from-html') as (
                    args, env, oracle, verifier, trial, output):
                async def verify():
                    if stdout is not None:
                        (trial / 'verifier/test-stdout.txt').write_text(stdout)
                    return SimpleNamespace(rewards={'reward': 1})
                verifier.verify.side_effect = verify
                with patch.object(runner, 'Verifier', return_value=verifier) as constructor:
                    row = asyncio.run(runner.run_task(args, {'name': 'filter-js-from-html'}, {},
                        environment_factory=lambda **kwargs: env))
                self.assertEqual(row['status'], 'passed' if passed else 'failed')
                self.assertEqual(row['rewards'], {'reward': 1})
                self.assertEqual(row['oracle'], 'passed')
                self.assertEqual(row['verifier'], 'passed')
                self.assertEqual(row['verification_audit']['browser_lifecycle_audit'], 'not_instrumented')
                self.assertEqual(constructor.call_args.kwargs['override_env'],
                                 {'PYTEST_ADDOPTS': '-s -v', 'PYTHONUNBUFFERED': '1'})
                self.assertEqual(row['verifier_observation_env'], constructor.call_args.kwargs['override_env'])
                env.start.assert_awaited_once()
                env.stop.assert_awaited_once()
                self.assertTrue(row['kill_confirmed'])
                self.assertEqual(json.loads((trial / 'result.json').read_text()), row)

    def test_browser_observation_does_not_change_other_verifiers(self):
        with self.fixture() as (args, env, oracle, verifier, trial, output):
            with patch.object(runner, 'Verifier', return_value=verifier) as constructor:
                row = asyncio.run(runner.run_task(args, {'name': 'sample'}, {},
                    environment_factory=lambda **kwargs: env))
            self.assertNotIn('override_env', constructor.call_args.kwargs)
            self.assertNotIn('verification_audit', row)
            self.assertNotIn('verifier_observation_env', row)
            self.assertEqual(row['status'], 'passed')

    def test_repaired_generic_browser_requires_supervised_lifecycle_and_keeps_reward(self):
        for lifecycle, reward, passed in ((None, 1, False), ('malformed', 1, False),
                ('{"remaining_owned_children": [123]}\n', 1, False),
                ('{"remaining_owned_children": []}\n', 1, True),
                ('{"remaining_owned_children": []}\n', 0, False)):
            with self.subTest(lifecycle=lifecycle, reward=reward), self.fixture(task_name='filter-js-from-html') as (
                    args, env, oracle, verifier, trial, output):
                args.oracle_mode = 'repaired'
                async def verify():
                    (trial / 'verifier/test-stdout.txt').write_text(
                        'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n')
                    if lifecycle is not None:
                        (trial / 'verifier/browser-lifecycle.jsonl').write_text(lifecycle)
                    return SimpleNamespace(rewards={'reward': reward})
                verifier.verify.side_effect = verify
                with patch.object(runner, 'prepare_repaired_task', return_value=args.source / 'repaired'), \
                        patch.object(runner, 'Verifier', return_value=verifier) as constructor:
                    row = asyncio.run(runner.run_task(args, {'name': 'filter-js-from-html'}, {},
                        environment_factory=lambda **kwargs: env))
                self.assertEqual(row['status'], 'passed' if passed else 'failed')
                self.assertEqual(row['rewards'], {'reward': reward})
                self.assertEqual(row['oracle_mode'], 'repaired')
                self.assertEqual(constructor.call_args.kwargs['override_env'], {
                    'PYTEST_ADDOPTS': '-s -v', 'PYTHONUNBUFFERED': '1',
                    'SE_CHROME_PATH': '/usr/local/lib/sandproof/guest_chromium.py'})
                self.assertEqual(row['verifier_repair_env'], {
                    'SE_CHROME_PATH': '/usr/local/lib/sandproof/guest_chromium.py'})
                self.assertNotEqual(row['verification_audit']['browser_lifecycle_audit'], 'not_instrumented')
                env.stop.assert_awaited_once()

    def test_fixed_base_browser_still_requires_lifecycle_evidence(self):
        with self.fixture(task_name='filter-js-from-html') as (args, env, oracle, verifier, trial, output):
            async def verify():
                (trial / 'verifier/test-stdout.txt').write_text(
                    'Total batches to test: 1\nRUN BATCH 0 (contains 1 tests)\n  No alert detected\n')
                (trial / 'verifier/browser-lifecycle.jsonl').write_text('malformed')
                return SimpleNamespace(rewards={'reward': 1})
            verifier.verify.side_effect = verify
            row = asyncio.run(runner.run_task(args, {'name': 'filter-js-from-html'}, {}))
            self.assertEqual(row['rewards'], {'reward': 1})
            self.assertEqual(row['verifier'], 'passed')
            self.assertEqual(row['status'], 'failed')
            self.assertEqual(row['verification_audit']['browser_lifecycle_audit'], 'failed')
            env.stop.assert_awaited_once()

    def test_post_score_probe_is_opt_in_and_preserves_original_failure(self):
        from benchmarks.failure_diagnostics import CYTHON_COMMAND
        for enabled, reward, name in ((True, 0, 'build-cython-ext'), (False, 0, 'build-cython-ext'),
                                      (True, 1, 'build-cython-ext'), (True, 0, 'sample')):
            with self.subTest(enabled=enabled, reward=reward, name=name), self.fixture(task_name=name) as (
                    args, env, oracle, verifier, trial, output):
                args.failure_diagnostics = enabled
                verifier.verify.return_value = SimpleNamespace(rewards={'reward': reward})
                env.exec.return_value = SimpleNamespace(stdout='nested assertion\n', stderr='', return_code=1)
                row = asyncio.run(runner.run_task(args, {'name': name}, {}))
                expected = enabled and reward == 0 and name == 'build-cython-ext'
                calls = [c for c in env.exec.call_args_list if c.args[0] == CYTHON_COMMAND]
                self.assertEqual(len(calls), int(expected))
                self.assertEqual(row['rewards'], {'reward': reward})
                self.assertEqual(row['status'], 'passed' if reward else 'failed')
                self.assertEqual(row['verifier'], 'passed' if reward else 'failed')
                self.assertEqual(row['oracle'], 'passed')
                if expected:
                    self.assertEqual(row['failure_diagnostic']['return_code'], 1)
                    self.assertEqual((trial / 'failure-diagnostic.txt').read_text(), 'nested assertion\n')
                    self.assertEqual(calls[0].kwargs, {'cwd': '/app/pyknotid', 'timeout_sec': 60})
                env.stop.assert_awaited_once()

    def test_gpt2_allocation_failure_is_diagnostic_only(self):
        from benchmarks.failure_diagnostics import GPT2_COMMAND
        for enabled, reward in ((True, 0), (False, 0), (True, 1)):
            with self.subTest(enabled=enabled, reward=reward), self.fixture(task_name='gpt2-codegolf') as (
                    args, env, oracle, verifier, trial, output):
                args.failure_diagnostics = enabled
                verifier.verify.return_value = SimpleNamespace(rewards={'reward': reward})
                env.exec.return_value = SimpleNamespace(stdout='allocation failed\n', stderr='', return_code=77)
                row = asyncio.run(runner.run_task(args, {'name': 'gpt2-codegolf'}, {}))
                calls = [c for c in env.exec.call_args_list if c.args[0] == GPT2_COMMAND]
                self.assertEqual(len(calls), int(enabled and reward == 0))
                self.assertEqual(row['rewards'], {'reward': reward})
                self.assertEqual(row['status'], 'passed' if reward else 'failed')
                self.assertEqual(row['verifier'], 'passed' if reward else 'failed')
                if calls:
                    self.assertEqual(row['failure_diagnostic']['return_code'], 77)
                    self.assertEqual(calls[0].kwargs, {'cwd': '/app', 'timeout_sec': 60})
                env.stop.assert_awaited_once()

    def test_failed_diagnostic_does_not_replace_original_score_or_skip_cleanup(self):
        from benchmarks.failure_diagnostics import CYTHON_COMMAND
        for failure in (TimeoutError('private detail'), OSError('private detail')):
            with self.subTest(failure=type(failure).__name__), self.fixture(task_name='build-cython-ext') as (
                    args, env, oracle, verifier, trial, output):
                args.failure_diagnostics = True
                verifier.verify.return_value = SimpleNamespace(rewards={'reward': 0})
                async def execute(command, **kwargs):
                    if command == CYTHON_COMMAND:
                        raise failure
                    return SimpleNamespace(stdout='', stderr='', return_code=0)
                env.exec.side_effect = execute
                row = asyncio.run(runner.run_task(args, {'name': 'build-cython-ext'}, {}))
                self.assertEqual(row['rewards'], {'reward': 0})
                self.assertEqual(row['status'], 'failed')
                self.assertEqual(row['failure_diagnostic']['error_type'], type(failure).__name__)
                self.assertNotIn('private detail', (trial / 'failure-diagnostic.json').read_text())
                env.stop.assert_awaited_once()

    def test_cancelled_diagnostic_preserves_score_and_still_cleans_original_vm(self):
        from benchmarks.failure_diagnostics import CYTHON_COMMAND
        with self.fixture(task_name='build-cython-ext') as (args, env, oracle, verifier, trial, output):
            args.failure_diagnostics = True
            verifier.verify.return_value = SimpleNamespace(rewards={'reward': 0})
            async def cancel():
                entered = asyncio.Event()
                async def execute(command, **kwargs):
                    if command == CYTHON_COMMAND:
                        entered.set()
                        await asyncio.Future()
                    return SimpleNamespace(stdout='', stderr='', return_code=0)
                env.exec.side_effect = execute
                running = asyncio.create_task(runner.run_task(args, {'name': 'build-cython-ext'}, {}))
                await asyncio.wait_for(entered.wait(), 1)
                running.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await running
            asyncio.run(cancel())
            row = json.loads((trial / 'result.json').read_text())
            self.assertEqual(row['rewards'], {'reward': 0})
            self.assertEqual(row['verifier'], 'failed')
            self.assertEqual(row['failure_diagnostic']['status'], 'interrupted')
            self.assertTrue(row['kill_confirmed'])
            env.stop.assert_awaited_once()

    def test_post_score_output_uses_trial_budget_and_still_cleans_up(self):
        from benchmarks.sdk_environment import SdkEnvironment
        with self.fixture(task_name='build-cython-ext') as (args, env, oracle, verifier, trial, output):
            args.failure_diagnostics = True
            env.trial_paths = SimpleNamespace(trial_dir=trial)
            env.artifact_budget = ArtifactBudget(dict(max_bytes=8, max_entries=8, max_depth=1, max_file_bytes=8))
            env.retain_text = lambda path, content: SdkEnvironment.retain_text(env, path, content)
            verifier.verify.return_value = SimpleNamespace(rewards={'reward': 0})
            async def execute(command, **kwargs):
                return SimpleNamespace(stdout='' if command.startswith('dpkg-query') else '123456789',
                                       stderr='', return_code=1)
            env.exec.side_effect = execute
            row = asyncio.run(runner.run_task(args, {'name': 'build-cython-ext'}, {}))
            self.assertEqual(row['rewards'], {'reward': 0})
            self.assertEqual(row['status'], 'failed')
            self.assertFalse(row['evidence_complete'])
            self.assertEqual(row['failure_diagnostic']['error_type'], 'ArtifactBudgetExceeded')
            self.assertFalse((trial / 'failure-diagnostic.txt').exists())
            env.stop.assert_awaited_once()

    def test_local_report_enospc_never_skips_cleanup_or_reports_success(self):
        for filename in ("error.txt", "events.json", "result.json"):
            with self.subTest(filename=filename), self.fixture(
                write_failure=filename,
                oracle_error=TimeoutError("oracle deadline") if filename == "error.txt" else None,
            ) as (args, env, oracle, verifier, trial, output):
                row = asyncio.run(runner.run_task(args, {"name": "sample"}, {}))
                env.start.assert_awaited_once()
                env.stop.assert_awaited_once()
                oracle.run.assert_awaited_once()
                self.assertTrue(row["kill_confirmed"])
                self.assertEqual(row["report_errors"][filename]["errno"], errno.ENOSPC)
                self.assertIn(row["status"], ("timeout", "failed"))
                self.assertEqual(json.loads(output.getvalue().splitlines()[-1]), row)
                if filename == "result.json":
                    self.assertFalse((trial / filename).exists())
                else:
                    self.assertEqual(json.loads((trial / "result.json").read_text()), row)
                if filename == "error.txt":
                    verifier.verify.assert_not_awaited()
                    self.assertEqual(row["oracle"], "timeout")
                else:
                    self.assertEqual(row["rewards"], {"reward": 1})
                    self.assertEqual(row["verifier"], "passed")

    def test_generic_log_failure_is_reported_without_masking_stage_or_cleanup(self):
        with self.fixture() as (args, env, oracle, verifier, trial, output):
            env.build_id = "a" * 32
            env.build_result = {"state": "Ready"}
            env.build_log_error = "OSError"
            env.template = {"templateId": "tpl-" + "b" * 32}
            env.build_dispatched = True
            env.submission_key = "c" * 32
            row = asyncio.run(runner.run_task(args, {"name": "sample"}, {},
                                            environment_factory=lambda **_kwargs: env))
            self.assertEqual(row["build_log_error"], "OSError")
            self.assertEqual(row["build"], {"state": "Ready"})
            self.assertEqual(row["status"], "passed")
            self.assertTrue(row["kill_confirmed"])
            env.stop.assert_awaited_once()
            self.assertEqual(json.loads((trial / "result.json").read_text()), row)

    def test_actual_task_cancellation_records_interrupted_stage_after_cleanup(self):
        for stage in ("initialization", "oracle", "verifier"):
            with self.subTest(stage=stage), self.fixture() as (args, env, oracle, verifier, trial, output):
                async def cancel_at_stage():
                    entered = asyncio.Event()

                    async def stall(*args, **kwargs):
                        entered.set()
                        await asyncio.Future()

                    target = {"initialization": env.start, "oracle": oracle.run,
                              "verifier": verifier.verify}[stage]
                    target.side_effect = stall
                    running = asyncio.create_task(runner.run_task(args, {"name": "sample"}, {}))
                    await asyncio.wait_for(entered.wait(), 1)
                    running.cancel()
                    with self.assertRaises(asyncio.CancelledError):
                        await running

                asyncio.run(cancel_at_stage())
                row = json.loads((trial / "result.json").read_text())
                self.assertEqual(row["status"], "interrupted")
                self.assertEqual(row["stage"], stage)
                self.assertTrue(row["interrupted"])
                self.assertTrue(row["kill_confirmed"])
                env.start.assert_awaited_once()
                env.stop.assert_awaited_once()
                if stage != "initialization":
                    self.assertEqual(row[stage], "interrupted")
                if stage != "verifier":
                    verifier.verify.assert_not_awaited()
                    self.assertEqual(row["verifier"], "not_run")
                else:
                    self.assertEqual(row["oracle"], "passed")
                if stage == "initialization":
                    oracle.run.assert_not_awaited()
                    self.assertEqual(row["oracle"], "not_run")

    def test_keyboard_interrupt_is_preserved_after_recording_and_cleanup(self):
        with self.fixture(oracle_error=KeyboardInterrupt()) as (args, env, oracle, verifier, trial, output):
            with self.assertRaises(KeyboardInterrupt):
                asyncio.run(runner.run_task(args, {"name": "sample"}, {}))
            row = json.loads((trial / "result.json").read_text())
            self.assertEqual(row["status"], "interrupted")
            self.assertEqual(row["oracle"], "interrupted")
            self.assertTrue(row["kill_confirmed"])
            env.stop.assert_awaited_once()
            verifier.verify.assert_not_awaited()

    def test_cleanup_unknown_survives_interruption_and_local_report_failure(self):
        for cleanup_error in (RuntimeError("kill result unknown"), asyncio.CancelledError()):
            with self.subTest(error=type(cleanup_error).__name__), self.fixture(
                write_failure="events.json", oracle_error=asyncio.CancelledError(),
                cleanup_error=cleanup_error,
            ) as (args, env, oracle, verifier, trial, output):
                with self.assertRaises(asyncio.CancelledError):
                    asyncio.run(runner.run_task(args, {"name": "sample"}, {}))
                row = json.loads((trial / "result.json").read_text())
                self.assertEqual(row["status"], "cleanup_unknown")
                self.assertEqual(row["sandbox_id"], "sb-fixture")
                self.assertEqual(row["cleanup_error_type"], type(cleanup_error).__name__)
                self.assertFalse(row["kill_confirmed"])
                self.assertTrue(row["interrupted"])
                self.assertEqual(row["oracle"], "interrupted")
                self.assertEqual(row["report_errors"]["events.json"]["errno"], errno.ENOSPC)
                env.start.assert_awaited_once()
                env.stop.assert_awaited_once()
                oracle.run.assert_awaited_once()
                verifier.verify.assert_not_awaited()


if __name__ == "__main__":
    unittest.main()
