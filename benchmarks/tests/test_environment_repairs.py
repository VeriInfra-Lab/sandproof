"""Explicit cache-only image variants preserve original task/scoring boundaries."""
import asyncio
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
import tarfile
import tempfile
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks.environment_repairs import PRUNED_SOURCE, archive_repaired_context, repair_profile
from benchmarks.generic_environment import GenericEnvironment, build_request
from benchmarks.guest_pip_cache import prune
from benchmarks.run_generic import run_selected
from tools.benchmark_tasks import InvalidInput


def task(name='mteb-leaderboard'):
    lock = json.loads((Path(__file__).parents[1] / 'terminal-bench-2.0.lock.json').read_text())
    return next(entry for entry in lock['tasks'] if entry['name'] == name)


def config(entry):
    return SimpleNamespace(os=SimpleNamespace(value='linux'), gpus=0, tpu=None, mcp_servers=[],
        network_mode=SimpleNamespace(value='public'), allowed_hosts=[],
        docker_image=entry['upstream_image_reference'])


class EnvironmentRepairTests(unittest.TestCase):
    def test_context_pins_base_and_contains_only_explicit_cache_helper(self):
        for name in ('mteb-leaderboard', 'mteb-retrieve'):
            with self.subTest(task=name), tempfile.TemporaryDirectory() as tmp:
                entry = task(name)
                path = Path(tmp) / 'context.tar'
                record = archive_repaired_context(entry, path)
                with tarfile.open(path) as archive:
                    self.assertEqual(archive.getnames(), ['Dockerfile'])
                    dockerfile = archive.extractfile('Dockerfile').read()
                lines = dockerfile.decode().splitlines()
                self.assertEqual(lines[0], 'FROM ' + record['base_image'])
                self.assertIn('@sha256:', lines[0])
                command = json.loads(lines[1].removeprefix('RUN '))
                self.assertEqual(command[:4], ['/usr/local/bin/python3', '-I', '-S', '-c'])
                helper = (Path(__file__).parents[1] / 'guest_pip_cache.py').read_bytes()
                self.assertEqual(command[4].encode(), helper)
                self.assertEqual(record['helper_sha256'], hashlib.sha256(helper).hexdigest())
                self.assertEqual(record['dockerfile_sha256'], hashlib.sha256(dockerfile).hexdigest())
                self.assertEqual(record['context_sha256'], hashlib.sha256(path.read_bytes()).hexdigest())
                self.assertFalse(record['original_environment'])
                original = build_request(entry, config(entry), 'oci')
                repaired = build_request(entry, config(entry), PRUNED_SOURCE)
                self.assertEqual(original['resources'], repaired['resources'])
                self.assertEqual(original['timeoutSeconds'], repaired['timeoutSeconds'])
                self.assertEqual(original['kind'], 'oci')
                self.assertEqual(repaired['kind'], 'dockerfile')
                self.assertNotIn('image', repaired)

    def test_changed_task_image_and_mixed_selection_fail_before_dispatch(self):
        entry = task()
        for change in ({'name': 'unregistered'}, {'files_sha256': '0' * 64},
                       {'upstream_image_reference': 'example/substitute:latest'}):
            with self.subTest(change=change), self.assertRaises(InvalidInput):
                repair_profile(entry | change)
        cfg = config(entry)
        cfg.docker_image = 'example/substitute:latest'
        with self.assertRaises(InvalidInput):
            build_request(entry, cfg, PRUNED_SOURCE)
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(all=True, task=None, output=Path(tmp), oracle_mode='original',
                                   source_kind=PRUNED_SOURCE)
            lock = {'tasks': [entry, task('write-compressor')]}
            with patch('benchmarks.run_generic.run_task', AsyncMock()) as run:
                with self.assertRaises(InvalidInput):
                    asyncio.run(run_selected(args, lock))
                run.assert_not_awaited()
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_early_runner_failure_still_records_environment_variant(self):
        with tempfile.TemporaryDirectory() as tmp:
            entry = task()
            args = SimpleNamespace(all=True, task=None, output=Path(tmp),
                                   oracle_mode='original', source_kind=PRUNED_SOURCE)
            with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=RuntimeError)):
                rows = asyncio.run(run_selected(args, {'tasks': [entry], 'source': {}}))
            report = json.loads((Path(tmp) / 'full-set.json').read_text())
            self.assertEqual(rows[0]['status'], 'runner_error')
            self.assertEqual(rows[0]['oracle_mode'], 'original')
            self.assertEqual(rows[0]['environment_source_kind'], PRUNED_SOURCE)
            self.assertFalse(rows[0]['original_environment'])
            self.assertFalse(report['original_environment'])

    def test_repaired_context_has_durable_provenance_before_submit_and_upload(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = object.__new__(GenericEnvironment)
            env.entry, env.source_kind = task(), PRUNED_SOURCE
            env.task_env_config = config(env.entry)
            env.trial_paths = SimpleNamespace(trial_dir=root)
            env.submission_key = 'a' * 32
            env.capture_build_log = AsyncMock()
            template = {'templateId': 'tpl-' + 'b' * 32, 'revision': 'r1', 'cpuCount': 1,
                        'memoryMib': 4096, 'diskBytes': 10 * 1024**3, 'startup': {},
                        'imageDigest': 'sha256:' + 'c' * 64, 'kernelSha256': 'd' * 64,
                        'rootfsSha256': 'e' * 64}
            def request(method, path, data=None, **kwargs):
                intent = json.loads((root / 'build-intent.json').read_text())
                self.assertFalse(intent['original_environment'])
                self.assertEqual(intent['source_kind'], PRUNED_SOURCE)
                self.assertEqual(intent['environment_repair']['context_sha256'], intent['context_sha256'])
                if method == 'POST':
                    self.assertEqual(data['kind'], 'dockerfile')
                    self.assertEqual(kwargs['token'], env.submission_key)
                    return {'buildID': 'b' * 32, 'state': 'AwaitingInput'}
                if method == 'PUT':
                    uploaded = data.read()
                    self.assertEqual(len(uploaded), kwargs['size'])
                    self.assertEqual(hashlib.sha256(uploaded).hexdigest(), intent['context_sha256'])
                    return {'buildID': 'b' * 32, 'state': 'Building'}
                return {'buildID': 'b' * 32, 'state': 'Ready', 'template': template}
            env.client = SimpleNamespace(request=Mock(side_effect=request))
            asyncio.run(env.prepare_template())
            self.assertEqual([call.args[0] for call in env.client.request.call_args_list], ['POST', 'PUT', 'GET'])
            self.assertEqual(env.template, template)
            self.assertFalse(env.environment_repair['original_environment'])


class PipCacheHelperTests(unittest.TestCase):
    def test_only_installation_cache_is_removed_and_other_inputs_remain_identical(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache = root / 'root/.cache/pip'
            (cache / 'http-v2/a').mkdir(parents=True)
            (cache / 'wheels').mkdir()
            payload = {'http-v2/a/download.body': b'cached wheel', 'wheels/a.whl': b'wheel archive'}
            for name, data in payload.items():
                (cache / name).write_bytes(data)
            preserved = {'root/.cache/huggingface/model': b'original model weights',
                         'usr/local/lib/torch/libtorch_cpu.so': b'installed original library',
                         'solution/solve.sh': b'original solution', 'tests/test.sh': b'original verifier'}
            for name, data in preserved.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            result = prune(cache)
            self.assertEqual(result['removed_files'], len(payload))
            self.assertEqual(result['removed_logical_bytes'], sum(map(len, payload.values())))
            self.assertTrue(result['removed'])
            self.assertFalse(cache.exists())
            for name, data in preserved.items():
                self.assertEqual((root / name).read_bytes(), data)

    def test_unknown_cache_entries_and_symlinks_are_rejected_without_deletion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cache = root / 'root/.cache/pip'
            (cache / 'http').mkdir(parents=True)
            payload = cache / 'http/payload'
            payload.write_bytes(b'cache')
            unknown = cache / 'user-data'
            unknown.write_bytes(b'preserve')
            with self.assertRaisesRegex(ValueError, 'unrecognized'):
                prune(cache)
            self.assertEqual(unknown.read_bytes(), b'preserve')
            unknown.unlink()
            outside = root / 'model'
            outside.write_bytes(b'preserve model')
            link = cache / 'http/linked'
            link.symlink_to(outside)
            with self.assertRaisesRegex(ValueError, 'regular cache'):
                prune(cache)
            self.assertEqual(payload.read_bytes(), b'cache')
            self.assertEqual(outside.read_bytes(), b'preserve model')
            link.unlink()
            cache.rename(root / 'original-cache')
            cache.symlink_to(root / 'original-cache', target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'real directories'):
                prune(cache)
            self.assertEqual((root / 'original-cache/http/payload').read_bytes(), b'cache')


if __name__ == '__main__':
    unittest.main()
