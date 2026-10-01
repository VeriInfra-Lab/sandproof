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
    def fixture(self, *, write_failure=None, oracle_error=None, cleanup_error=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            task = SimpleNamespace(
                name="sample", instruction="fixture",
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
                yield args, env, oracle, verifier, root / "sample", output

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
