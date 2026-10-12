"""Probe failure/cancellation must retain identities and attempt every cleanup."""
import asyncio
import copy
import json
import sqlite3
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'
sys.path.insert(0, str(SYSTEM))
try:
    import template_resource_pressure as pressure
    import harbor_generic_chain as chain
finally:
    sys.path.remove(str(SYSTEM))
from benchmarks.run_generic import run_selected
from tools.generic_capacity import GIB, MIB, plan
from tools.template_acceptance import budgets
from tools.template_build_client import BuildClient
from tools.template_build_service import BuildHTTP
from tools.template_build_store import BuildStore
from tools.tests.test_template_build_service import limits as store_limits
from tools.tests import test_template_build_retention as retention_fixtures
from tools.tests.template_build_fixture import PRIVATE_TEST_KEY
from benchmarks.tests.build_http_fixture import PublicBuildHTTP, serve_http


def deployment():
    entries = [{'name': str(i), 'requirements': {'cpus': 1, 'memory_mib': 2048,
                'disk_mib': 10240, 'build_timeout_sec': 600}} for i in range(4)]
    return budgets(plan(entries, 1, 32 * GIB, 64 * MIB), 32 * GIB, 64 * MIB)


def templates():
    return [{'template': {'templateId': 'tpl-' + str(i) * 32, 'cpuCount': 1,
                          'memoryMib': 2048, 'diskBytes': 10 * GIB}} for i in (1, 2)]


class PressureBuildWindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Existing fixture compiles the real Rust authority with locked deps.
        retention_fixtures.RetentionTests.setUpClass.__func__(cls)

    def service(self, root):
        directory = root / 'builds'
        directory.mkdir(mode=0o700)
        config = retention_fixtures.bounded_config(self.helper)
        limits = store_limits() | {'max_jobs': 4, 'max_jobs_per_tenant': 4,
                                   'max_pending': 1, 'max_pending_per_tenant': 1}
        store = BuildStore(directory, limits, bounded=config,
                           free_bytes=lambda: 1000000 * MIB)
        self.addCleanup(store.close)
        backend = serve_http(self, BuildHTTP(('127.0.0.1', 0), store, PRIVATE_TEST_KEY))
        public = serve_http(self, PublicBuildHTTP(backend, {'synthetic': 1}))
        return store, BuildClient('http://127.0.0.1:' + str(public.server_port), 'synthetic')

    def temporary_root(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        return Path(temporary.name).resolve()

    def test_windowed_real_http_reaches_budget_rejection_with_durable_distinct_intents(self):
        root = self.temporary_root()
        store, client = self.service(root)
        request = {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 60}
        checkpoints = []
        def checkpoint(label, value):
            saved = json.loads((root / ('pressure-' + label + '-intent.json')).read_text())
            self.assertEqual(saved['requestWindow'], value['requestWindow'])
            self.assertEqual(saved['idempotency_key'], value['token'])
            self.assertEqual(saved['request'], request)
            checkpoints.append((label, value, len(store.rows())))
        result = asyncio.run(pressure.submit_build(root, client, request, 'windowed', 'first',
            lambda value: checkpoint('first', value)))
        self.assertEqual(result['state'], 'Queued')
        with self.assertRaisesRegex(RuntimeError, '^BuildBudgetExceeded$'):
            asyncio.run(pressure.submit_build(root, client, request, 'windowed', 'rejected',
                lambda value: checkpoint('rejected', value)))
        self.assertEqual([c[2] for c in checkpoints], [0, 1])
        self.assertNotEqual(checkpoints[0][1]['requestWindow'], checkpoints[1][1]['requestWindow'])
        self.assertEqual(len(store.rows()), 1)

    def test_removed_mode_cannot_open_a_window_or_submit(self):
        root = self.temporary_root()
        client = Mock()
        with self.assertRaisesRegex(ValueError, 'only windowed'):
            asyncio.run(pressure.submit_build(root, client,
                {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 60},
                'legacy-unbounded', 'legacy', Mock()))
        client.request.assert_not_called()
        self.assertEqual(list(root.iterdir()), [])

    def test_lost_public_reply_keeps_original_intent_and_never_resubmits(self):
        root = self.temporary_root()
        store, client = self.service(root)
        original = client.request
        posts = []
        def request(method, path, body=None, **kwargs):
            result = original(method, path, body, **kwargs)
            if path == '/sandproof/template-builds':
                posts.append((body, kwargs))
                raise TimeoutError('lost original response')
            return result
        client.request = request
        with self.assertRaises(TimeoutError):
            asyncio.run(pressure.submit_build(root, client,
                {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 60},
                'windowed', 'lost', lambda _intent: None))
        saved = json.loads((root / 'pressure-lost-intent.json').read_text())
        self.assertEqual(len(posts), 1)
        self.assertEqual(len(store.rows()), 1)
        self.assertEqual(posts[0][0]['requestWindow'], saved['requestWindow'])
        self.assertEqual(posts[0][1]['token'], saved['idempotency_key'])

    def test_private_lookup_and_retired_anchor_settlement_stay_bound_to_original(self):
        with self.assertRaisesRegex(ValueError, 'original private window is mandatory'):
            pressure.private_vm_path('job', None)
        self.assertEqual(pressure.private_vm_path('job', 'original-window'),
                         '/sandproof/internal-build-vms/job?requestWindow=original-window')
        database = self.temporary_root() / 'builds.db'
        expected = {'requestWindow': 'original-window', 'sandboxID': 'original-sandbox'}
        with sqlite3.connect(database) as connection:
            connection.execute('CREATE TABLE jobs(id TEXT,state TEXT,private_window TEXT,builder_settled TEXT,private_ack INTEGER)')
            connection.execute('INSERT INTO jobs VALUES(?,?,?,?,?)',
                ('job', 'Failed', json.dumps('original-window'), json.dumps(expected), 1))
        before = database.read_bytes()
        self.assertEqual(pressure.private_settlement(database, 'job', 'original-window', 'original-sandbox'), expected)
        self.assertEqual(database.read_bytes(), before)
        for column, value in (('state', 'Building'), ('private_ack', 0), ('private_window', json.dumps('other')),
                              ('builder_settled', None), ('builder_settled', json.dumps(expected | {'sandboxID': 'other'}))):
            with self.subTest(column=column, value=value):
                with sqlite3.connect(database) as connection:
                    connection.execute('UPDATE jobs SET ' + column + '=?', (value,))
                with self.assertRaises(ValueError):
                    pressure.private_settlement(database, 'job', 'original-window', 'original-sandbox')
                with sqlite3.connect(database) as connection:
                    connection.execute('UPDATE jobs SET state=?,private_window=?,builder_settled=?,private_ack=?',
                                       ('Failed', json.dumps('original-window'), json.dumps(expected), 1))


class PressureReplayTests(unittest.TestCase):
    def exercise_marker(self, executions):
        async def run(root):
            marker = root / "marker with ' quote"
            async def execute(command, **kwargs):
                for _ in range(executions):
                    process = await asyncio.create_subprocess_exec(
                        'sh', '-c', command, stdin=asyncio.subprocess.PIPE,
                        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE)
                    stdout, stderr = await process.communicate(b'')
                    self.assertEqual(process.returncode, 0)
                    self.assertEqual(stderr, b'')
                    kwargs['on_stdout'](stdout.decode())
                return SimpleNamespace()
            sandbox = SimpleNamespace(
                commands=SimpleNamespace(run=AsyncMock(side_effect=execute)),
                files=SimpleNamespace(read=AsyncMock(
                    side_effect=lambda path, **_: Path(path).read_text())))
            tracked = []
            await pressure.hold_input(sandbox, str(marker), tracked)
            sandbox.commands.run.assert_awaited_once()
            self.assertEqual(marker.read_text(), 'once\n' * executions)
            if executions == 1:
                await pressure.verify_no_replay(tracked)
            else:
                with self.assertRaises(AssertionError):
                    await pressure.verify_no_replay(tracked)
        with tempfile.TemporaryDirectory() as temporary:
            asyncio.run(run(Path(temporary)))

    def test_single_command_execution_passes_replay_check(self):
        self.exercise_marker(1)

    def test_duplicate_command_execution_fails_replay_check(self):
        self.exercise_marker(2)


class PressureFailureTests(unittest.TestCase):
    def test_repaired_ready_templates_and_pressure_pass_retain_failed_batch_and_mode(self):
        names = ['build-cython-ext', 'build-pmars']
        trials = [row | {'name': name, 'status': 'failed', 'oracle_mode': 'repaired',
                         'rewards': {'reward': 0.0}, 'build': {'state': 'Ready'},
                         'exact_cleanup': 'passed'} for name, row in zip(names, templates())]
        original = copy.deepcopy(trials)
        with tempfile.TemporaryDirectory() as temporary:
            args = SimpleNamespace(source=Path(temporary), output=Path(temporary), all=False,
                task=names, source_kind='oci', build_url='http://127.0.0.1:1', oracle_mode='repaired',
                completion_audit=Mock(return_value={'exact_cleanup': 'passed'}))
            lock = {'source': {}, 'tasks': [{'name': name} for name in names]}
            with patch('benchmarks.run_generic.run_task', AsyncMock(side_effect=trials)) as run:
                rows = asyncio.run(run_selected(args, lock))
            report = json.loads((args.output / 'full-set.json').read_text())
        self.assertEqual(run.await_count, 2)
        self.assertEqual(rows, original)
        self.assertEqual(report['oracle_mode'], 'repaired')
        self.assertEqual(report['tasks'], original)
        self.assertTrue(pressure.ready_for_pressure(rows))
        outcome = chain.repetition_outcome(rows, names, oracle_mode=report['oracle_mode'],
            resource_pressure=True, pressure_report={'status': 'passed'})
        self.assertFalse(outcome['execution_passed'])
        self.assertEqual(outcome['oracle_mode'], 'repaired')
        self.assertTrue(outcome['original_verifier'])
        self.assertFalse(outcome['original_oracle_verifier'])
        self.assertEqual(rows, original)
        for mode in ('original', 'repaired'):
            passed_rows = [row | {'status': 'passed', 'oracle_mode': mode, 'rewards': {'reward': 1.0}}
                           for row in rows]
            with self.subTest(mode=mode):
                passed = chain.repetition_outcome(passed_rows, names, oracle_mode=mode,
                    resource_pressure=True, pressure_report={'status': 'passed'})
                self.assertTrue(passed['execution_passed'])
                self.assertEqual(passed['original_oracle_verifier'], mode == 'original')
                self.assertFalse(chain.repetition_outcome(passed_rows, names, oracle_mode=mode,
                    resource_pressure=True, pressure_report={'status': 'failed'})['execution_passed'])

    def test_slow_read_reobserves_original_builder_without_mutating_or_extending_deadline(self):
        vm = {'session': {'known': True}, 'sandboxID': 'original'}
        read = AsyncMock(side_effect=[TimeoutError('disk preparation'), vm])
        status = AsyncMock(return_value={'state': 'Running'})
        with patch.object(pressure.time, 'time', return_value=10), \
             patch.object(pressure.asyncio, 'sleep', AsyncMock()):
            result = asyncio.run(pressure.wait_builder_session(read, status, 20, Mock(poll=lambda: None)))
        self.assertIs(result, vm)
        self.assertEqual(read.await_count, 2)
        status.assert_awaited_once_with()

    def test_builder_read_retry_retains_original_deadline_and_does_not_hide_terminal_or_other_errors(self):
        for value in ('deadline', 'Ready', 'unexpected'):
            with self.subTest(value=value):
                read = AsyncMock(side_effect=ValueError('bad identity') if value == 'unexpected' else TimeoutError())
                status = AsyncMock(return_value={'state': 'Ready'})
                clock = [10, 21] if value == 'deadline' else [10, 10]
                with patch.object(pressure.time, 'time', side_effect=clock), \
                     patch.object(pressure.asyncio, 'sleep', AsyncMock()), \
                     self.assertRaises(ValueError if value == 'unexpected' else AssertionError):
                    asyncio.run(pressure.wait_builder_session(read, status, 20, Mock(poll=lambda: None)))
                read.assert_awaited_once_with()
                self.assertEqual(status.await_count, int(value == 'Ready'))

    def test_pressure_requires_ready_and_exact_cleanup_but_never_changes_failed_scores(self):
        rows = [row | {'status': 'failed', 'rewards': {'reward': 0.0}, 'build': {'state': 'Ready'},
                      'exact_cleanup': 'passed'} for row in templates()]
        original = copy.deepcopy(rows)
        self.assertTrue(pressure.ready_for_pressure(rows))
        self.assertEqual(rows, original)
        for changes in ({'exact_cleanup': 'unknown'}, {'build': {'state': 'Failed'}},
                        {'template': None}, {'build': None}):
            with self.subTest(changes=changes):
                self.assertFalse(pressure.ready_for_pressure([rows[0], rows[1] | changes]))
        for count in (0, 1, 3):
            self.assertFalse(pressure.ready_for_pressure([rows[0]] * count))

    def test_four_instance_cpu_memory_and_disk_checked_before_dispatch(self):
        shape = {'cpus': 1, 'memory_mib': 2048, 'disk_mib': 10240}
        pressure.validate_cohort([shape, shape], deployment())
        for field, value in (('cpus', 2), ('memory_mib', 8192), ('disk_mib', 20480)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                pressure.validate_cohort([shape, shape | {field: value}], deployment())

    def invoke(self, root, creates, *, install_error=None, audit=None):
        services = Mock(spec=['sdk_url'], sdk_url='http://127.0.0.1:1')
        build_config = root / 'build-config.json'
        parsed_config = object()
        async def run():
            with patch.object(pressure.AsyncSandbox, 'create', AsyncMock(side_effect=creates)) as create, \
                 patch.object(pressure, 'clean', AsyncMock()), \
                 patch.object(pressure, 'install', AsyncMock(side_effect=install_error)), \
                 patch.object(pressure, 'configuration', return_value=parsed_config) as configuration, \
                 patch.object(pressure, 'Backend', return_value=Mock(spec=['request'])) as backend:
                try:
                    await pressure.exercise(root=root, services=services,
                        key='synthetic', build_config=build_config, rows=templates(), deployment=deployment(),
                        supervisor=Mock(), restart=Mock(), source_entry={},
                        build_mode='windowed',
                        audit_shape=lambda env: {'disk_inode': int(env.sandbox_id)},
                        completion_audit=audit or Mock(return_value={'exact_cleanup': 'passed'}))
                finally:
                    self.calls = create.await_count
                    configuration.assert_called_once_with(build_config)
                    backend.assert_called_once_with(parsed_config)
                    for call in create.await_args_list:
                        self.assertEqual(call.kwargs['api_url'], services.sdk_url)
                        self.assertEqual(call.kwargs['sandbox_url'], services.sdk_url)
                        self.assertEqual(call.kwargs['api_key'], 'synthetic')
                        self.assertEqual(call.kwargs['retries'], 0)
        return asyncio.run(run())

    def sandbox(self, index, kill_error=None):
        return SimpleNamespace(sandbox_id=str(index), kill=AsyncMock(return_value=True, side_effect=kill_error))

    def test_lost_create_reply_does_not_retry_and_cleans_known_instance(self):
        first = self.sandbox(1)
        audit = Mock(return_value={'exact_cleanup': 'passed'})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(TimeoutError):
                self.invoke(root, [first, TimeoutError('lost reply')], audit=audit)
            report = json.loads((root / 'resource-pressure.json').read_text())
        self.assertEqual(self.calls, 2)
        first.kill.assert_awaited_once()
        audit.assert_called_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['error_type'], 'TimeoutError')
        self.assertEqual(report['identities'][0]['sandbox_id'], '1')

    def test_cancelled_probe_still_attempts_all_four_original_stops(self):
        cohort = [self.sandbox(i) for i in range(1, 5)]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(asyncio.CancelledError):
                self.invoke(root, cohort, install_error=asyncio.CancelledError())
            report = json.loads((root / 'resource-pressure.json').read_text())
        for sandbox in cohort:
            sandbox.kill.assert_awaited_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['error_type'], 'CancelledError')
        self.assertEqual(len(report['identities']), 4)

    def test_one_stop_error_does_not_skip_other_stops_or_external_audit(self):
        cohort = [self.sandbox(i, TimeoutError() if i == 4 else None) for i in range(1, 5)]
        audit = Mock(return_value={'exact_cleanup': 'passed'})
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(BaseExceptionGroup):
                self.invoke(root, cohort, install_error=ValueError('package failure'), audit=audit)
            report = json.loads((root / 'resource-pressure.json').read_text())
        for sandbox in cohort:
            sandbox.kill.assert_awaited_once()
        audit.assert_called_once()
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(report['exact_cleanup'], 'passed')
        self.assertEqual(report['error_type'], 'ValueError')
        self.assertEqual(report['cleanup_error_types'], ['TimeoutError'])


if __name__ == '__main__':
    unittest.main()
