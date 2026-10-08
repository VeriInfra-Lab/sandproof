"""Prepared CI dispatch/report wiring; substitutes are not VM/scoring evidence."""
import asyncio
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks import run_generic, run_local
from benchmarks.prepared_ci_environment import PreparedCiEnvironment
from benchmarks.ci_verifier import manifest_path, MODE, REQUIREMENTS_SHA256
from tools.benchmark_tasks import InvalidInput

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
try:
    from harbor_generic_chain import network_transport_evidence
finally:
    sys.path.remove(str(SYSTEM))


class PreparedCiWiringTests(unittest.TestCase):
    def test_missing_or_noncallable_verifier_rejects_before_dispatch_and_report(self):
        for factory in (None, 'not-a-function', False, {}):
            with self.subTest(factory=factory), tempfile.TemporaryDirectory() as temporary:
                args = SimpleNamespace(all=False, task=['sample'], oracle_mode='original',
                    source_kind='prepared-ci', ci_fixtures={'sample': object()}, output=Path(temporary),
                    verifier_task_factory=factory)
                lock = {'dataset': 'terminal-bench@2.0', 'tasks': [{'name': 'sample'}]}
                with patch.object(run_generic, 'run_task', AsyncMock()) as run:
                    with self.assertRaisesRegex(InvalidInput, 'scoring-entry variant factory'):
                        asyncio.run(run_generic.run_selected(args, lock))
                run.assert_not_awaited()
                self.assertFalse((args.output / 'full-set.json').exists())

    def test_prepared_full_set_report_does_not_claim_original_verifier(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = SimpleNamespace(all=False, task=['sample'], oracle_mode='original',
                source_kind='prepared-ci', ci_fixtures={'sample': object()}, output=Path(temporary),
                verifier_task_factory=lambda *args: None)
            lock = {'dataset': 'terminal-bench@2.0', 'tasks': [{'name': 'sample'}],
                    'source': {'commit': 'a' * 40}}
            row = {'name': 'sample', 'status': 'failed', 'oracle': 'passed', 'verifier': 'failed',
                   'create_dispatched': False, 'build_dispatched': False}
            with patch.object(run_generic, 'run_task', AsyncMock(return_value=row)):
                asyncio.run(run_generic.run_selected(args, lock))
            report = json.loads((args.output / 'full-set.json').read_bytes())
            self.assertFalse(report['original_environment'])
            self.assertIn('CI scoring-entry variant', report['scope'])
            self.assertIn('original test assertions', report['scope'])
            self.assertNotIn('and original verifier', report['scope'])

    def test_prepared_network_report_is_disabled_and_does_not_query_a_proxy(self):
        self.assertEqual(network_transport_evidence(None, prepared=True),
                         {'mode': 'disabled', 'public_network': False})
        proxy = SimpleNamespace(evidence=Mock(return_value={'mode': 'proxy'}))
        with self.assertRaises(ValueError):
            network_transport_evidence(proxy, prepared=True)
        proxy.evidence.assert_not_called()
        self.assertEqual(network_transport_evidence(None), {'mode': 'direct_public_ipv4'})
        self.assertEqual(network_transport_evidence(proxy), {'mode': 'proxy'})

    def test_guest_preflight_success_updates_checkpoint_after_real_result(self):
        env = object.__new__(PreparedCiEnvironment)
        env.fixture = SimpleNamespace(guest_preflight_command=Mock(return_value='clean-boundary'),
            evidence=Mock(return_value={'guest_preflight': 'required_not_run', 'mode': 'prepared-ci'}))
        env.checkpoint = Mock()
        async def successful_exec(*args, **kwargs):
            self.assertEqual(env.guest_preflight, 'running')
            env.checkpoint.assert_not_called()
            return SimpleNamespace(return_code=0)
        env.exec = AsyncMock(side_effect=successful_exec)
        with patch('benchmarks.generic_environment.GenericEnvironment.start', AsyncMock()):
            asyncio.run(env.start())
        self.assertEqual(env.guest_preflight, 'passed')
        env.checkpoint.assert_called_once_with('prepared-ci-fixture.json',
            {'guest_preflight': 'passed', 'mode': 'prepared-ci'})

    def test_failed_or_unknown_preflight_never_records_passed_checkpoint(self):
        for outcome in (SimpleNamespace(return_code=1), TimeoutError('unknown command'), asyncio.CancelledError()):
            with self.subTest(outcome=type(outcome).__name__):
                env = object.__new__(PreparedCiEnvironment)
                env.fixture = SimpleNamespace(guest_preflight_command=Mock(return_value='clean-boundary'))
                env.checkpoint = Mock()
                env.exec = AsyncMock(return_value=outcome) if not isinstance(outcome, BaseException) else AsyncMock(side_effect=outcome)
                with patch('benchmarks.generic_environment.GenericEnvironment.start', AsyncMock()):
                    with self.assertRaises((ValueError, TimeoutError, asyncio.CancelledError)):
                        asyncio.run(env.start())
                self.assertEqual(env.guest_preflight, 'failed')
                env.checkpoint.assert_not_called()

    def test_create_failure_leaves_preflight_not_run(self):
        env = object.__new__(PreparedCiEnvironment)
        env.exec = AsyncMock()
        with patch('benchmarks.generic_environment.GenericEnvironment.start', AsyncMock(side_effect=TimeoutError())):
            with self.assertRaises(TimeoutError):
                asyncio.run(env.start())
        self.assertEqual(env.guest_preflight, 'not_run')
        env.exec.assert_not_awaited()

    def test_trial_row_binds_scoring_manifest_and_preserves_original_oracle(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = SimpleNamespace(name='sample', instruction='synthetic instruction',
                paths=SimpleNamespace(task_dir=root / 'source/sample', environment_dir=root / 'source/sample/environment'),
                config=SimpleNamespace(environment=SimpleNamespace(build_timeout_sec=1),
                    agent=SimpleNamespace(timeout_sec=1), verifier=SimpleNamespace(timeout_sec=1)))
            derived = SimpleNamespace(paths=SimpleNamespace(task_dir=root / 'derived'))
            evidence = {'mode': MODE, 'source_launcher_sha256': 'a' * 64,
                        'launcher_sha256': 'b' * 64, 'requirements_sha256': REQUIREMENTS_SHA256}
            def factory(task, entry, destination):
                self.assertIs(task, original)
                manifest_path(destination).write_text(json.dumps(evidence))
                return derived
            env = SimpleNamespace(start=AsyncMock(), sandbox_id='sb-synthetic', create_dispatched=True,
                kill_confirmed=False, events=[], guest_preflight='passed',
                exec=AsyncMock(return_value=SimpleNamespace(stdout='', stderr='', return_code=0)))
            env.retain_text = lambda path, data: path.write_text(data)
            async def stop():
                env.kill_confirmed = True
            env.stop = AsyncMock(side_effect=stop)
            oracle = SimpleNamespace(run=AsyncMock())
            verifier = SimpleNamespace(verify=AsyncMock(return_value=SimpleNamespace(rewards={'reward': 1})))
            args = SimpleNamespace(source=root, output=root, api_url='http://127.0.0.1:1',
                                   verifier_task_factory=factory)
            with (patch.object(run_local, 'Task', return_value=original),
                  patch.object(run_local, 'LocalBaseEnvironment', return_value=env),
                  patch.object(run_local, 'OracleAgent', return_value=oracle) as oracle_constructor,
                  patch.object(run_local, 'Verifier', return_value=verifier) as verifier_constructor,
                  patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'synthetic'}),
                  contextlib.redirect_stdout(io.StringIO())):
                row = asyncio.run(run_local.run_task(args, {'name': 'sample'}, {}))
            self.assertEqual(row['verifier_variant'], evidence)
            self.assertEqual(row['verifier_variant_manifest_sha256'], hashlib.sha256(
                manifest_path(root / 'sample/ci-verifier-task').read_bytes()).hexdigest())
            self.assertEqual(row['guest_preflight'], 'passed')
            self.assertFalse(row['original_verifier'])
            self.assertIs(oracle_constructor.call_args.kwargs['task_dir'], original.paths.task_dir)
            self.assertIs(verifier_constructor.call_args.kwargs['task'], derived)
            env.stop.assert_awaited_once()
            self.assertTrue(row['kill_confirmed'])


if __name__ == '__main__':
    unittest.main()
