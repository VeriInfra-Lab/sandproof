"""Regression cases for interrupted commands and best-effort diagnostics."""
import asyncio
import contextlib
import io
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


if __name__ == "__main__":
    unittest.main()
