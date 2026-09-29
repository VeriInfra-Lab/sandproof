"""Local-runner rejection and cleanup regressions; no task or VM execution."""
import asyncio
from pathlib import Path
from types import SimpleNamespace
import tempfile
import json
import unittest
from unittest.mock import AsyncMock, patch

from benchmarks.run_local import InvalidInput, LocalBaseEnvironment, input_path, local_url, load_recipes


class LocalRunnerTests(unittest.TestCase):
    def test_task_omission_is_rejected_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipes = Path(tmp) / 'recipes.json'
            recipes.write_text(json.dumps({}))
            with patch('benchmarks.run_local.RECIPES', recipes), self.assertRaises(InvalidInput):
                load_recipes({'tasks': [{'name': 'required-task'}]}, Path(tmp))

    def test_base_substitution_is_rejected_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            recipes = Path(tmp) / 'recipes.json'
            recipes.write_text(json.dumps({'task': {'base': 'ubuntu-24.04'}}))
            lock = {'tasks': [{'name': 'task', 'dockerfile_from_lines': ['python:3.13-slim-bookworm']}]}
            with patch('benchmarks.run_local.RECIPES', recipes), self.assertRaises(InvalidInput):
                load_recipes(lock, Path(tmp))

    def test_never_defaults_to_cloud(self):
        for url in ('https://api.e2b.dev', 'http://127.0.0.1.evil',
                    'http://user:secret@localhost', 'http://localhost/x',
                    'file:///tmp/service', 'http://localhost?key=secret'):
            with self.subTest(url=url), self.assertRaises(InvalidInput):
                local_url(url)
        self.assertEqual(local_url('http://127.0.0.1:1234'), 'http://127.0.0.1:1234')

    def test_recipe_cannot_upload_private_phase(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'environment'
            root.mkdir()
            for value in ('../solution/solve.sh', '/etc/passwd', '../../tests/test.sh'):
                with self.subTest(value=value), self.assertRaises(InvalidInput):
                    input_path(root, value)
            (root / 'link').symlink_to('/etc/passwd')
            with self.assertRaises(InvalidInput):
                input_path(root, 'link')

    def test_failed_kill_keeps_sandbox_identity(self):
        env = object.__new__(LocalBaseEnvironment)
        env._sandbox = SimpleNamespace(kill=AsyncMock(side_effect=RuntimeError('unknown')))
        original = env._sandbox
        with self.assertRaises(RuntimeError):
            asyncio.run(env.stop())
        self.assertIs(env._sandbox, original)
        env._sandbox.kill.assert_awaited_once()

    def test_negative_kill_is_not_success(self):
        env = object.__new__(LocalBaseEnvironment)
        env._sandbox = SimpleNamespace(kill=AsyncMock(return_value=False))
        with self.assertRaises(RuntimeError):
            asyncio.run(env.stop())
        self.assertFalse(env.kill_confirmed)
        self.assertIsNotNone(env._sandbox)

    def test_guest_download_cannot_escape_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = object.__new__(LocalBaseEnvironment)
            env.trial_paths = SimpleNamespace(trial_dir=Path(tmp) / 'trial')
            env.trial_paths.trial_dir.mkdir()
            env._sandbox = SimpleNamespace(files=SimpleNamespace(read=AsyncMock()))
            with self.assertRaises(InvalidInput):
                asyncio.run(env.download_file('/logs/verifier/a', Path(tmp) / 'outside'))
            env._sandbox.files.read.assert_not_awaited()

    def test_guest_listing_cannot_inject_host_path(self):
        from e2b import FileType
        env = object.__new__(LocalBaseEnvironment)
        env._sandbox = SimpleNamespace(files=SimpleNamespace(list=AsyncMock(
            return_value=[SimpleNamespace(path='/logs/verifier/../../etc/passwd', type=FileType.FILE)])))
        env.download_file = AsyncMock()
        with self.assertRaises(InvalidInput):
            asyncio.run(env.download_dir('/logs/verifier', '/tmp/output'))
        env.download_file.assert_not_awaited()


if __name__ == '__main__':
    unittest.main()
