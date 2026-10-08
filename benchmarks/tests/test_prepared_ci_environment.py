"""Prepared CI keeps real SDK/reconciliation boundaries; these are component tests."""
import asyncio
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks.prepared_ci_environment import PreparedCiEnvironment


def prepared_fixture():
    return SimpleNamespace(
        verify_artifacts=Mock(), evidence=Mock(return_value={'mode': 'prepared-ci'}),
        guest_preflight_command=Mock(return_value='test -x /opt/sandproof-ci-verifier/bin/python'),
        startup={'workdir': '/app', 'user': 'root', 'env_vars': {}, 'argv': []},
        template_descriptor=Mock(return_value={
            'template_id': 'ci-' + 'b' * 32, 'revision': 'r1',
            'kernel': {'sha256': 'c' * 64}, 'rootfs': {'sha256': 'd' * 64},
            'resources': {'cpu_count': 1, 'memory_mib': 2048, 'disk_bytes': 10 * 1024**3,
                          'host_memory_bytes': 2304 * 1024**2}}))


class PreparedCiEnvironmentTests(unittest.TestCase):
    def test_prepare_never_calls_build_api_and_pins_reconciliation_binding(self):
        env = object.__new__(PreparedCiEnvironment)
        env.fixture, env.checkpoint = prepared_fixture(), Mock()
        env.client, env.build_dispatched = Mock(), False
        asyncio.run(env.prepare_template())
        env.client.request.assert_not_called()
        self.assertFalse(env.build_dispatched)
        self.assertEqual(env.build_id, 'b' * 32)
        self.assertEqual(env.template['templateId'], 'ci-' + env.build_id)
        env.fixture.verify_artifacts.assert_called_once()

    def test_publication_or_unbound_static_prefix_is_rejected_before_create(self):
        for identity in ('tpl-' + 'b' * 32, 'ci-' + 'b' * 31, 'other-' + 'b' * 32):
            with self.subTest(identity=identity):
                env = object.__new__(PreparedCiEnvironment)
                env.fixture, env.checkpoint = prepared_fixture(), Mock()
                env.fixture.template_descriptor.return_value['template_id'] = identity
                env.build_dispatched = False
                with (self.assertRaisesRegex(ValueError, 'explicit static template binding'),
                      patch('benchmarks.generic_environment.AsyncSandbox.create') as create):
                    asyncio.run(env.prepare_template())
                create.assert_not_called()
                self.assertFalse(env.build_dispatched)
                env.checkpoint.assert_not_called()

    def test_changed_fixture_rejects_before_any_create(self):
        env = object.__new__(PreparedCiEnvironment)
        env.fixture, env.checkpoint = prepared_fixture(), Mock()
        env.fixture.verify_artifacts.side_effect = ValueError('changed pinned fixture')
        with self.assertRaises(ValueError), patch('benchmarks.generic_environment.AsyncSandbox.create') as create:
            asyncio.run(env.prepare_template())
        create.assert_not_called()

    def test_create_is_network_disabled_and_lost_reply_is_not_retried(self):
        from harbor.models.task.config import NetworkMode
        with tempfile.TemporaryDirectory() as temporary:
            env = object.__new__(PreparedCiEnvironment)
            env._network_policy = SimpleNamespace(network_mode=NetworkMode.PUBLIC)
            env.fixture, env.client = prepared_fixture(), Mock()
            env.entry = {'requirements': {'build_timeout_sec': 600}, 'dataset': 'terminal-bench-2.0'}
            env.trial_paths = SimpleNamespace(trial_dir=Path(temporary))
            env.environment_name, env.sdk_options = 'fixture', {'api_key': 'synthetic'}
            env._startup_env, env.sandbox_id = Mock(return_value={}), None
            with patch('benchmarks.generic_environment.AsyncSandbox.create',
                       AsyncMock(side_effect=TimeoutError('lost create receipt'))) as create:
                with self.assertRaises(TimeoutError):
                    asyncio.run(env.start())
            create.assert_awaited_once()
            self.assertIs(create.call_args.kwargs['allow_internet_access'], False)
            self.assertEqual(create.call_args.kwargs['metadata']['build'], 'b' * 32)
            self.assertTrue(env.create_dispatched)
            self.assertIsNotNone(env.runtime_intent)
            env.client.request.assert_not_called()

    def test_force_build_cannot_make_prepared_fixture_online(self):
        env = object.__new__(PreparedCiEnvironment)
        with self.assertRaises(ValueError):
            asyncio.run(env.start(force_build=True))

    def test_guest_preflight_failure_does_not_fake_success(self):
        env = object.__new__(PreparedCiEnvironment)
        env.fixture = prepared_fixture()
        env.exec = AsyncMock(return_value=SimpleNamespace(return_code=1))
        with patch('benchmarks.generic_environment.GenericEnvironment.start', AsyncMock()) as start:
            with self.assertRaises(ValueError):
                asyncio.run(env.start())
        start.assert_awaited_once()


if __name__ == '__main__':
    unittest.main()
