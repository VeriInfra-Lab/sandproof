"""Generic original-input, resource, dispatch and phase boundaries; no VMs."""
import asyncio
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace
import tarfile
import tempfile
import threading
import unittest
from unittest.mock import AsyncMock, Mock, patch

from benchmarks.generic_environment import archive_context, build_request, GenericEnvironment, validate_template
from benchmarks.run_generic import run_selected
from benchmarks.artifact_budget import ArtifactBudget, DEFAULT_ARTIFACT_LIMITS
from tools.benchmark_tasks import InvalidInput


def config():
    return SimpleNamespace(os=SimpleNamespace(value='linux'), gpus=0, tpu=None, mcp_servers=[],
        network_mode=SimpleNamespace(value='public'), allowed_hosts=[], docker_image='example/task:fixed')


def entry():
    return {'name': 'sample', 'source_path': 'sample', 'files_sha256': 'a' * 64,
            'upstream_image_reference': 'example/task:fixed',
            'requirements': {'cpus': 2, 'memory_mib': 4096, 'disk_mib': 10240, 'build_timeout_sec': 600.0}}


def template():
    # The Rust serializer verifies this exact fixture in template_wire.rs.
    fixture = Path(__file__).resolve().parents[2] / 'tests/compat/template-view.json'
    return json.loads(fixture.read_text())


class GenericInputTests(unittest.TestCase):
    def test_lost_create_receipt_has_durable_intent_before_dispatch_and_no_retry(self):
        from harbor.models.task.config import NetworkMode
        with tempfile.TemporaryDirectory() as tmp:
            env = object.__new__(GenericEnvironment)
            env._network_policy = SimpleNamespace(network_mode=NetworkMode.PUBLIC)
            env.template, env.entry = template(), entry()
            env.environment_name, env.build_id = 'sample', 'a' * 32
            env.sdk_options = {'api_key': 'synthetic'}
            env.prepare_template = AsyncMock()
            env._startup_env = Mock(return_value={})
            env.trial_paths = SimpleNamespace(trial_dir=Path(tmp))
            env.sandbox_id = None
            async def lost(**kwargs):
                intent = json.loads((Path(tmp) / 'runtime-intent.json').read_text())
                self.assertEqual(intent['metadata'], kwargs['metadata'])
                self.assertEqual(intent['template'], env.template)
                raise TimeoutError('lost create reply')
            with patch('benchmarks.generic_environment.AsyncSandbox.create', AsyncMock(side_effect=lost)) as create:
                with self.assertRaises(TimeoutError):
                    asyncio.run(env.start())
            create.assert_awaited_once()
            self.assertTrue(env.create_dispatched)
            self.assertIsNone(env.sandbox_id)

    def test_expired_initialization_never_dispatches_runtime_create(self):
        from harbor.models.task.config import NetworkMode
        env = object.__new__(GenericEnvironment)
        env._network_policy = SimpleNamespace(network_mode=NetworkMode.PUBLIC)
        env.entry, env.create_dispatched = entry(), False
        env.prepare_template = AsyncMock()
        with patch('benchmarks.generic_environment.AsyncSandbox.create', AsyncMock()) as create, \
             patch('benchmarks.generic_environment.time', SimpleNamespace(monotonic=Mock(side_effect=[1000, 1600]))):
            with self.assertRaises(TimeoutError):
                asyncio.run(env.start())
        create.assert_not_awaited()
        self.assertFalse(env.create_dispatched)

    def test_shared_rust_response_requires_canonical_memory_field(self):
        self.assertEqual(validate_template(template(), entry()), template())
        wrong = template()
        wrong['memoryMiB'] = wrong.pop('memoryMib')
        with self.assertRaises(InvalidInput):
            validate_template(wrong, entry())

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
            with patch('benchmarks.generic_environment.AsyncSandbox.create', AsyncMock(return_value=sandbox)) as create, \
                 patch('benchmarks.generic_environment.time', SimpleNamespace(monotonic=Mock(side_effect=[1000, 1120]))):
                asyncio.run(env.start())
            self.assertEqual(env._workdir, '/final')
            self.assertEqual(env.default_user, 'app')
            self.assertEqual(create.call_args.kwargs['template'], env.template['templateId'])
            self.assertEqual(create.call_args.kwargs['timeout'], 86400)
            self.assertEqual(create.call_args.kwargs['request_timeout'], 480)
            self.assertNotIn('cpu_count', create.call_args.kwargs)
            self.assertEqual(env.sandbox_id, 'sb-exact')
            intent = json.loads((Path(tmp) / 'runtime-intent.json').read_text())
            self.assertEqual(intent['metadata'], create.call_args.kwargs['metadata'])
            self.assertEqual(intent['template'], env.template)
            env.exec.assert_awaited_once_with('test ! -e /solution && test ! -e /tests', cwd='/', user='root', timeout_sec=30)

    def test_build_diagnostics_keep_terminal_result_and_original_identity(self):
        for state in ('Ready', 'Failed', 'CleanupUnknown'):
            for log_failure in (False, True):
                with self.subTest(state=state, log_failure=log_failure), tempfile.TemporaryDirectory() as tmp:
                    env = object.__new__(GenericEnvironment)
                    env.entry, env.source_kind = entry(), 'oci'
                    env.task_env_config = config()
                    env.trial_paths = SimpleNamespace(trial_dir=Path(tmp))
                    env.artifact_budget = ArtifactBudget(DEFAULT_ARTIFACT_LIMITS)
                    env.submission_key = 'a' * 32
                    env.build_log_error = None
                    job = 'b' * 32
                    outcome = {'buildID': job, 'state': state, 'failure': 'original build failure',
                               'template': template() if state == 'Ready' else None}
                    log = OSError('must not retain credential-bearing exception text') if log_failure else {
                        'offset': 0, 'nextOffset': 16384, 'text': '\x00' * 16384}
                    env.client = SimpleNamespace(request=Mock(side_effect=[
                        {'buildID': job, 'state': 'Queued'}, outcome, log]))
                    if state == 'Ready':
                        asyncio.run(env.prepare_template())
                        self.assertEqual(env.template, template())
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'original build failure'):
                            asyncio.run(env.prepare_template())
                    self.assertEqual(env.build_result, outcome)
                    self.assertEqual(env.build_id, job)
                    self.assertEqual(env.build_log_error, 'OSError' if log_failure else None)
                    self.assertEqual([call.args[:2] for call in env.client.request.call_args_list], [
                        ('POST', '/sandproof/template-builds'),
                        ('GET', '/sandproof/template-builds/' + job),
                        ('GET', '/sandproof/template-builds/' + job + '/logs?offset=0&limit=65536')])
                    if not log_failure:
                        self.assertEqual(json.loads((Path(tmp) / 'build-log.json').read_text()),
                                         log | {'complete': state in ('Ready', 'Failed'), 'truncated': False})


class BuildLogTests(unittest.IsolatedAsyncioTestCase):
    async def capture(self, root, pages, *, cap=8, page_size=4, budget=None):
        env = object.__new__(GenericEnvironment)
        env.trial_paths = SimpleNamespace(trial_dir=root)
        env.artifact_budget = budget or ArtifactBudget(DEFAULT_ARTIFACT_LIMITS)
        env.client = SimpleNamespace(request=Mock(side_effect=pages))
        env.build_result = {'state': 'Failed'}
        with patch('benchmarks.generic_environment.BUILD_LOG_BYTES', cap), \
             patch('benchmarks.generic_environment.BUILD_LOG_PAGE', page_size):
            await env.capture_build_log('/build')
        return env, json.loads((root / 'build-log.json').read_text())

    async def test_raw_offsets_keep_tail_and_charge_json_expansion(self):
        with tempfile.TemporaryDirectory() as tmp:
            env, log = await self.capture(Path(tmp), [
                {'offset': 0, 'nextOffset': 4, 'text': '\ufffd' * 4},
                {'offset': 4, 'nextOffset': 8, 'text': 'tail'},
                {'offset': 8, 'nextOffset': 8, 'text': ''}])
            self.assertEqual(log['text'], '\ufffd' * 4 + 'tail')
            self.assertTrue(log['complete'])
            self.assertFalse(log['truncated'])
            self.assertEqual([c.args[1] for c in env.client.request.call_args_list], [
                '/build/logs?offset=0&limit=4', '/build/logs?offset=4&limit=4',
                '/build/logs?offset=8&limit=1'])
            self.assertEqual(env.artifact_budget.bytes, (Path(tmp) / 'build-log.json').stat().st_size)

    async def test_over_cap_is_explicit_and_does_not_retain_extra_byte(self):
        with tempfile.TemporaryDirectory() as tmp:
            env, log = await self.capture(Path(tmp), [
                {'offset': 0, 'nextOffset': 4, 'text': 'head'},
                {'offset': 4, 'nextOffset': 8, 'text': 'tail'},
                {'offset': 8, 'nextOffset': 9, 'text': '!'}])
            self.assertEqual(log['text'], 'headtail')
            self.assertEqual(log['nextOffset'], 8)
            self.assertTrue(log['truncated'])
            self.assertFalse(log['complete'])
            self.assertEqual(env.client.request.call_count, 3)

    async def test_later_network_or_malformed_page_keeps_first_page(self):
        for bad in (OSError('credential-bearing error'),
                    {'offset': 0, 'nextOffset': 4, 'text': 'back'},
                    {'offset': 4, 'nextOffset': 9, 'text': 'large'},
                    {'offset': 4, 'nextOffset': 4, 'text': 'invalid'},
                    {'offset': 4, 'nextOffset': 5, 'text': 'too long'}):
            with self.subTest(bad=type(bad).__name__), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises((OSError, InvalidInput)):
                    await self.capture(Path(tmp), [{'offset': 0, 'nextOffset': 4, 'text': 'head'}, bad])
                log = json.loads((Path(tmp) / 'build-log.json').read_text())
                self.assertEqual(log, {'offset': 0, 'nextOffset': 4, 'text': 'head',
                                       'complete': False, 'truncated': False})

    async def test_trial_budget_cannot_be_bypassed_by_build_logs(self):
        with tempfile.TemporaryDirectory() as tmp:
            budget = ArtifactBudget(dict(max_bytes=8, max_entries=1, max_depth=1, max_file_bytes=8))
            with self.assertRaises(InvalidInput):
                await self.capture(Path(tmp), [{'offset': 0, 'nextOffset': 0, 'text': ''}], budget=budget)
            self.assertIsNotNone(budget.failure)
            self.assertFalse((Path(tmp) / 'build-log.json').exists())

    async def test_cancelled_read_preserves_partial_log_and_propagates(self):
        with tempfile.TemporaryDirectory() as tmp:
            entered = asyncio.Event()
            calls = []
            async def request(_call, method, path):
                calls.append(path)
                if len(calls) == 1:
                    return {'offset': 0, 'nextOffset': 4, 'text': 'head'}
                entered.set()
                await asyncio.Future()
            with patch('benchmarks.generic_environment.asyncio.to_thread', request):
                running = asyncio.create_task(self.capture(Path(tmp), []))
                await asyncio.wait_for(entered.wait(), 1)
                running.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await running
            log = json.loads((Path(tmp) / 'build-log.json').read_text())
            self.assertEqual(log['text'], 'head')
            self.assertFalse(log['complete'])
            self.assertEqual(len(calls), 2)


class GenericSuiteTests(unittest.TestCase):
    def test_cancel_during_external_audit_waits_for_same_audit_before_propagating(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True, task=None,
                source_kind='oci', build_url='http://127.0.0.1:1')
            lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b')]}
            finished = threading.Event()
            async def exercise():
                entered = asyncio.Event()
                release = threading.Event()
                loop = asyncio.get_running_loop()
                def audit(row):
                    loop.call_soon_threadsafe(entered.set)
                    if not release.wait(2):
                        raise TimeoutError('test did not release audit')
                    finished.set()
                    return {'exact_cleanup': 'passed'}
                args.completion_audit = Mock(side_effect=audit)
                running = asyncio.create_task(run_selected(args, lock))
                await asyncio.wait_for(entered.wait(), 1)
                running.cancel()
                await asyncio.sleep(0)
                self.assertFalse(running.done())
                running.cancel()
                release.set()
                with self.assertRaises(asyncio.CancelledError):
                    await running
                self.assertTrue(finished.is_set())
            with patch('benchmarks.run_generic.run_task', AsyncMock(return_value={
                    'name': 'a', 'status': 'failed', 'execution_status': 'timeout',
                    'error_type': 'TimeoutError'})) as trial:
                asyncio.run(exercise())
            trial.assert_awaited_once()
            args.completion_audit.assert_called_once()
            rows = json.loads((Path(tmp) / 'full-set.json').read_text())['tasks']
            self.assertEqual(rows[0]['execution_status'], 'timeout')
            self.assertEqual(rows[0]['error_type'], 'TimeoutError')
            self.assertTrue(rows[0]['interrupted'])
            self.assertEqual(rows[0]['exact_cleanup'], 'passed')
            self.assertEqual(rows[1]['status'], 'not_run')

    def test_recovered_create_failure_keeps_score_and_continues_only_after_physical_audit(self):
        for exact in ('passed', 'unknown'):
            with self.subTest(exact=exact), tempfile.TemporaryDirectory() as tmp:
                audit = Mock(return_value={'exact_cleanup': exact,
                    'sandbox_id': 'sb-original', 'runtime_recovery': {'status': 'destroyed'}})
                args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True, task=None,
                    source_kind='oci', build_url='http://127.0.0.1:1', completion_audit=audit)
                lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b')]}
                first = {'name': 'a', 'status': 'create_unknown', 'execution_status': 'timeout',
                         'kill_confirmed': False, 'oracle': 'not_run', 'verifier': 'not_run'}
                with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=[first,
                        {'name': 'b', 'status': 'passed'}])) as trial:
                    rows = asyncio.run(run_selected(args, lock))
                self.assertEqual(trial.await_count, 2 if exact == 'passed' else 1)
                self.assertEqual(rows[0]['status'], 'create_unknown')
                self.assertEqual(rows[0]['execution_status'], 'timeout')
                self.assertFalse(rows[0]['kill_confirmed'])
                self.assertEqual(rows[0]['verifier'], 'not_run')

    def test_cancelled_trial_still_calls_external_recovery_without_starting_next_task(self):
        with tempfile.TemporaryDirectory() as tmp:
            audit = Mock(return_value={'exact_cleanup': 'passed'})
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True, task=None,
                source_kind='oci', build_url='http://127.0.0.1:1', completion_audit=audit)
            lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b')]}
            async def interrupted(*args, **kwargs):
                result = Path(tmp) / 'a/result.json'
                result.parent.mkdir()
                kwargs['result_row'].update(name='a', status='create_unknown',
                    interrupted=True, runtime_intent={'original': True})
                result.write_text(json.dumps(kwargs['result_row']))
                raise asyncio.CancelledError()
            with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=interrupted)) as trial:
                with self.assertRaises(asyncio.CancelledError):
                    asyncio.run(run_selected(args, lock))
            trial.assert_awaited_once()
            audit.assert_called_once()
            self.assertEqual(audit.call_args.args[0]['runtime_intent'], {'original': True})
            rows = json.loads((Path(tmp) / 'full-set.json').read_text())['tasks']
            self.assertTrue(rows[0]['interrupted'])
            self.assertEqual(rows[0]['exact_cleanup'], 'passed')
            self.assertEqual(rows[1]['status'], 'not_run')

    def test_repair_mode_is_explicit_and_preserved_in_complete_report(self):
        for mode in ('original', 'repaired'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=False,
                    task=['build-pmars'], source_kind='oci', build_url='http://127.0.0.1:1')
                if mode == 'repaired':
                    args.oracle_mode = mode
                lock = {'source': {}, 'tasks': [{'name': 'build-pmars'}, {'name': 'sample'}]}
                result = {'name': 'build-pmars', 'status': 'passed', 'oracle_mode': mode}
                with patch('benchmarks.run_generic.run_task', AsyncMock(return_value=result)) as trial:
                    rows = asyncio.run(run_selected(args, lock))
                self.assertEqual(getattr(trial.call_args.args[0], 'oracle_mode', 'original'), mode)
                report = json.loads((Path(tmp) / 'full-set.json').read_text())
                self.assertEqual(report['oracle_mode'], mode)
                self.assertEqual(report['tasks'], rows)
                self.assertEqual(rows[1]['status'], 'not_run')

    def test_mixed_or_unknown_repairs_reject_before_any_trial_or_report(self):
        for mode in ('repaired', 'unknown'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as tmp:
                args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True,
                    task=None, oracle_mode=mode, source_kind='oci', build_url='http://127.0.0.1:1')
                lock = {'source': {}, 'tasks': [{'name': 'build-pmars'}, {'name': 'sample'}]}
                with patch('benchmarks.run_generic.run_task', AsyncMock()) as trial:
                    with self.assertRaises(InvalidInput):
                        asyncio.run(run_selected(args, lock))
                trial.assert_not_awaited()
                self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_original_failed_build_audit_allows_next_task_without_erasing_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            audit = Mock(return_value={'exact_cleanup': 'passed', 'build': {'state': 'Failed'}})
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True, task=None,
                                   source_kind='oci', build_url='http://127.0.0.1:1', completion_audit=audit)
            lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b')]}
            results = [{'name': name, 'status': 'failed', 'build_dispatched': True,
                        'build': {'state': 'Building'}} for name in ('a', 'b')]
            with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=results)) as trial:
                rows = asyncio.run(run_selected(args, lock))
            self.assertEqual(trial.await_count, 2)
            self.assertEqual(audit.call_count, 2)
            self.assertTrue(all(r['status'] == 'failed' and r['exact_cleanup'] == 'passed' for r in rows))
            self.assertEqual(json.loads((Path(tmp) / 'full-set.json').read_text())['tasks'], rows)

    def test_failed_external_audit_stops_suite_even_with_successful_sdk_receipt(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=True, task=None,
                                   source_kind='oci', build_url='http://127.0.0.1:1',
                                   completion_audit=Mock(side_effect=RuntimeError('live VM')))
            lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b')]}
            with patch('benchmarks.run_generic.run_task', AsyncMock(return_value={
                    'name': 'a', 'status': 'passed', 'cleanup_acknowledged': True})) as trial:
                rows = asyncio.run(run_selected(args, lock))
            trial.assert_awaited_once()
            self.assertEqual(rows[0]['status'], 'cleanup_unknown')
            self.assertEqual(rows[1]['reason'], 'previous_identity_requires_audit')
            self.assertEqual(json.loads((Path(tmp) / 'a/result.json').read_text()), rows[0])

    def test_unknown_identity_stops_suite_without_dropping_other_tasks(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=False, task=['a', 'b'],
                                   source_kind='dockerfile', build_url='http://127.0.0.1:1')
            lock = {'source': {}, 'tasks': [{'name': name} for name in ('a', 'b', 'c')]}
            first = {'name': 'a', 'status': 'create_unknown', 'sandbox_id': None}
            with patch('benchmarks.run_generic.run_task', AsyncMock(return_value=first)) as trial:
                rows = asyncio.run(run_selected(args, lock))
            trial.assert_awaited_once()
            self.assertEqual(len(rows), 3)
            self.assertEqual(rows[1]['reason'], 'previous_identity_requires_audit')
            self.assertEqual(rows[2]['reason'], 'not_selected')
            self.assertEqual(json.loads((Path(tmp) / 'full-set.json').read_text())['tasks'], rows)

    def test_cancelled_trial_with_missing_report_is_interrupted_not_not_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            args = SimpleNamespace(source=Path(tmp), output=Path(tmp), all=False, task=['a'],
                                   source_kind='dockerfile', build_url='http://127.0.0.1:1')
            lock = {'source': {}, 'tasks': [{'name': 'a'}, {'name': 'b'}]}
            with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=asyncio.CancelledError())):
                with self.assertRaises(asyncio.CancelledError):
                    asyncio.run(run_selected(args, lock))
            rows = json.loads((Path(tmp) / 'full-set.json').read_text())['tasks']
            self.assertEqual(rows[0]['status'], 'interrupted')
            self.assertEqual(rows[0]['exact_cleanup'], 'unknown')
            self.assertEqual(rows[1]['status'], 'not_run')


if __name__ == '__main__':
    unittest.main()
