"""Generic original-input, resource, dispatch and phase boundaries; no VMs."""
import asyncio
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
import tarfile
import tempfile
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks.generic_environment import archive_context, build_request, GenericEnvironment, validate_template
from tools.benchmark_tasks import InvalidInput


def config():
    return SimpleNamespace(os=SimpleNamespace(value='linux'), gpus=0, tpu=None, mcp_servers=[],
        network_mode=SimpleNamespace(value='public'), allowed_hosts=[], docker_image='example/task:fixed')


def entry():
    return {'name': 'sample', 'source_path': 'sample', 'files_sha256': 'a' * 64,
            'upstream_image_reference': 'example/task:fixed',
            'requirements': {'cpus': 2, 'memory_mib': 4096, 'disk_mib': 10240, 'build_timeout_sec': 600.0}}


def template():
    return {'templateId': 'tpl-' + 'a' * 32, 'revision': 'r1', 'imageDigest': 'sha256:' + 'b' * 64,
            'kernelSha256': 'c' * 64, 'rootfsSha256': 'd' * 64,
            'cpuCount': 2, 'memoryMiB': 4096, 'diskBytes': 10240 * 1024**2,
            'startup': {'user': 'app', 'workdir': '/final', 'env_vars': {}, 'argv': []}}


class GenericInputTests(unittest.TestCase):
    def test_exact_multistage_context_excludes_private_phases(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            environment = root / 'sample/environment'
            environment.mkdir(parents=True)
            contents = {'Dockerfile': b'FROM ubuntu AS build\nCOPY payload /x\nFROM ubuntu\nCOPY --from=build /x /y\n',
                        'payload': b'exact\x00content'}
            specs = []
            for name, data in contents.items():
                (environment / name).write_bytes(data)
                specs.append({'path': 'environment/' + name, 'role': 'environment', 'mode': '100644',
                              'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
            (root / 'sample/solution').mkdir()
            (root / 'sample/solution/solve.sh').write_text('private')
            specs.append({'path': 'solution/solve.sh', 'role': 'oracle'})
            task = entry() | {'files': specs}
            archive_context(task, root, root / 'context.tar')
            with tarfile.open(root / 'context.tar') as archive:
                self.assertEqual(set(archive.getnames()), set(contents))
                for name, data in contents.items():
                    self.assertEqual(archive.extractfile(name).read(), data)
            (environment / 'payload').write_text('modified')
            with self.assertRaises(InvalidInput):
                archive_context(task, root, root / 'changed.tar')

    def test_explicit_resources_are_not_free_disk_or_sdk_create_fields(self):
        body = build_request(entry(), config(), 'dockerfile')
        self.assertEqual(body['resources'], {'cpuCount': 2, 'memoryMiB': 4096, 'diskMiB': 10240, 'minFreeMiB': 64})
        self.assertEqual(body['timeoutSeconds'], 600)
        self.assertNotIn('image', body)
        self.assertEqual(build_request(entry(), config(), 'oci')['image'], 'example/task:fixed')

    def test_unsupported_or_substituted_inputs_fail_before_dispatch(self):
        for field, value in [('gpus', 1), ('mcp_servers', ['service']), ('allowed_hosts', ['example.com'])]:
            with self.subTest(field=field):
                cfg = config()
                setattr(cfg, field, value)
                with self.assertRaises(InvalidInput):
                    build_request(entry(), cfg, 'dockerfile')
        cfg = config()
        cfg.docker_image = 'other:latest'
        with self.assertRaises(InvalidInput):
            build_request(entry(), cfg, 'oci')
        for field in ('diskBytes', 'templateId', 'rootfsSha256'):
            value = template()
            value[field] = 1 if field == 'diskBytes' else 'wrong'
            with self.subTest(field=field), self.assertRaises(InvalidInput):
                validate_template(value, entry())

    def test_unknown_post_records_same_intent_and_never_retries(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = object.__new__(GenericEnvironment)
            env.entry, env.source_kind = entry(), 'oci'
            env.task_env_config = config()
            env.trial_paths = SimpleNamespace(trial_dir=Path(tmp))
            env.submission_key = 'a' * 32
            env.client = SimpleNamespace(request=Mock(side_effect=OSError('lost response')))
            with self.assertRaises(OSError):
                asyncio.run(env.prepare_template())
            self.assertTrue(env.build_dispatched)
            env.client.request.assert_called_once()
            intent = json.loads((Path(tmp) / 'build-intent.json').read_text())
            self.assertEqual(intent['idempotency_key'], env.submission_key)
            self.assertEqual(env.client.request.call_args.kwargs['token'], env.submission_key)

    def test_create_selects_immutable_template_defaults_not_dockerfile_guess(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = object.__new__(GenericEnvironment)
            env._network_policy = SimpleNamespace(network_mode=SimpleNamespace(value='public'))
            # Property uses Harbor's actual NetworkMode, not a string fixture.
            from harbor.models.task.config import NetworkMode
            env._network_policy.network_mode = NetworkMode.PUBLIC
            env.template, env.entry = template(), entry()
            env.environment_name, env.build_id = 'sample', 'e' * 32
            env.sdk_options = {'api_key': 'synthetic', 'api_url': 'http://127.0.0.1:1'}
            env.prepare_template = AsyncMock()
            env.start_audit = None
            env._startup_env = Mock(return_value={})
            env.trial_paths = SimpleNamespace(trial_dir=Path(tmp))
            env.ensure_dirs, env._upload_environment_dir_after_start = AsyncMock(), AsyncMock()
            env.exec = AsyncMock(return_value=SimpleNamespace(return_code=0))
            sandbox = SimpleNamespace(sandbox_id='sb-exact', get_info=AsyncMock(return_value=SimpleNamespace(
                template_id=env.template['templateId'], cpu_count=2, memory_mb=4096)))
            with patch('benchmarks.generic_environment.AsyncSandbox.create', AsyncMock(return_value=sandbox)) as create:
                asyncio.run(env.start())
            self.assertEqual(env._workdir, '/final')
            self.assertEqual(env.default_user, 'app')
            self.assertEqual(create.call_args.kwargs['template'], env.template['templateId'])
            self.assertEqual(create.call_args.kwargs['timeout'], 86400)
            self.assertNotIn('cpu_count', create.call_args.kwargs)
            self.assertEqual(env.sandbox_id, 'sb-exact')
            env.exec.assert_awaited_once_with('test ! -e /solution && test ! -e /tests', cwd='/', user='root', timeout_sec=30)


if __name__ == '__main__':
    unittest.main()
