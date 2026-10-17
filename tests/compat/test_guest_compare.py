"""Paired probe comparison/orchestration and actual fixed SDK loopback calls."""
import copy
import argparse
import asyncio
from contextlib import ExitStack, redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import shlex
import stat
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch
from wsgiref.simple_server import make_server

from e2b import AsyncSandbox, Sandbox
from e2b.exceptions import FileNotFoundException
from e2b.sandbox.commands.command_handle import CommandExitException
from sdk_support import isolated_sdk_environment
from test_sdk_envd_wire import QuietHandler, SyntheticEnvd, ThreadedEnvd
from tools import guest_probe

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_guest_compare as probe
import sdk_observations as observations
import sdk_resource_observations as resources


def resource_report(expected, daemon, rss=8192):
    return dict(schema_version=1, expected=expected, daemon=daemon,
        sample=dict(rss_bytes=rss, reported_peak_rss_bytes=16384, threads=3, open_fds=9, elapsed_ns=100))


def environment_receipt():
    return dict(parent='/var/tmp', mountpoint='/', filesystem='ext4', block_size=4096,
        fragment_size=4096, name_max=255, mode=0o1777, hostname='sandproof-paired')


def rows():
    return [{'method': 'commands.run', 'args': ['fixed input'], 'kwargs': {'user': 'root'},
             'result': {'stdout': 'a中b', 'stderr': '', 'exit_code': 0, 'error': None},
             'callbacks': {'stdout': ['a', '中b'], 'stderr': []}},
            {'method': 'files.list', 'args': ['/tmp/shared'], 'kwargs': {'depth': 4},
             'result': [{'path': '/tmp/shared/a', 'mode': 1, 'modified_time': 'fixed', 'size': 4096},
                        {'path': '/tmp/shared/b', 'mode': 2, 'modified_time': 'fixed'}]},
            {'method': 'files.read', 'args': ['/tmp/shared/a'], 'kwargs': {'format': 'stream'},
             'result': {'base64': 'YWJj'}, 'transport_chunk_bytes': [1, 2]},
            {'method': 'commands.run', 'args': ['bad'], 'kwargs': {'cwd': '/tmp/missing'},
             'error': {'class': 'InvalidArgumentException', 'message': 'exact path /tmp/missing'}}]


class ComparisonTests(unittest.TestCase):
    def test_paired_cli_requires_every_pin_and_old_optional_cli_is_unchanged(self):
        parser = argparse.ArgumentParser()
        probe.guest_arguments(parser, required=True)
        flags = ['--template', '--guest-release', '--guest-manifest-sha256',
                 '--guest-configuration', '--guest-configuration-sha256']
        values = [part for flag in flags for part in (flag, 'explicit')]
        self.assertEqual(parser.parse_args(values).template, 'explicit')
        for index in range(0, len(values), 2):
            with self.subTest(missing=values[index]), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                parser.parse_args(values[:index] + values[index+2:])
        optional = argparse.ArgumentParser()
        probe.guest_arguments(optional)
        self.assertIsNone(optional.parse_args([]).template)

    def test_only_segmentation_is_ignored_and_directory_order_remains_exact(self):
        original = rows()
        other = copy.deepcopy(original)
        other[0]['callbacks']['stdout'] = ['a中', 'b']
        other[2]['transport_chunk_bytes'] = [3]
        probe.compare(original, other, 'same bytes')
        self.assertEqual(original, rows(), 'comparison must not rewrite evidence')
        mutations = [lambda r: r[0]['args'].append('extra input'),
                     lambda r: r[0].__setitem__('status', 'incomplete'),
                     lambda r: r[1]['result'].reverse(),
                     lambda r: r[0]['callbacks'].__setitem__('stdout', ['中ab']),
                     lambda r: r[0]['result'].__setitem__('exit_code', False),
                     lambda r: r[1]['result'][0].__setitem__('mode', True),
                     lambda r: r[1]['result'][0].__setitem__('modified_time', 'different'),
                     lambda r: r[1]['result'][0].__setitem__('size', 80),
                     lambda r: r[2]['result'].__setitem__('base64', 'YWJk'),
                     lambda r: r[3]['error'].__setitem__('message', 'different error'),
                     lambda r: r[3]['error'].__setitem__('class', 'OtherError')]
        for mutation in mutations:
            other = rows()
            mutation(other)
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                probe.compare(original, other, 'changed')
        duplicate = rows()
        duplicate[1]['result'].append(duplicate[1]['result'][0])
        with self.assertRaisesRegex(AssertionError, 'duplicate'):
            probe.compare(original, duplicate, 'duplicate')

    def test_capture_and_input_serialization_are_bounded_and_keep_fresh_streams(self):
        capture = probe.Capture()
        capture('中')
        with patch.object(observations, 'CONTENT_LIMIT', 3), self.assertRaises(AssertionError):
            capture('!')
        stream = io.BytesIO(bytes(range(256)))
        self.assertEqual(probe.value(stream), probe.value(bytes(range(256))))
        self.assertEqual(stream.tell(), 0)
        stream.read(1)
        with self.assertRaises(AssertionError):
            probe.value(stream)


class EnvironmentCommandTests(unittest.TestCase):
    def test_native_fixture_checks_actual_mount_before_hostname_and_never_rewrites_metadata(self):
        self.assertTrue(probe.plan()['directory'].startswith('/var/tmp/sandproof-paired-'))
        for initialize, mount, symlink, missing in (
                (True, 'ext4', False, False), (True, 'ext4', False, True),
                (False, 'ext4', False, False), (True, 'tmpfs', False, False),
                (True, 'ext4', True, False)):
            with self.subTest(initialize=initialize, mount=mount, symlink=symlink, missing=missing), ExitStack() as stack:
                command = shlex.split(probe.environment_command(initialize=initialize))
                self.assertEqual(command[:3], ['/usr/bin/python3', '-I', '-c'])
                # /tmp really is a different mount; only /var/tmp's longest
                # actual covering mount determines whether this fixture fits.
                mounts = ('1 0 8:0 / / rw - ext4 /dev/vda rw\n'
                          '2 1 0:1 / /tmp rw - tmpfs tmpfs rw\n')
                if mount == 'tmpfs':
                    mounts += '3 1 0:2 / /var/tmp rw - tmpfs tmpfs rw\n'
                stack.enter_context(patch('builtins.open', return_value=io.StringIO(mounts)))
                stack.enter_context(patch('os.lstat', return_value=SimpleNamespace(
                    st_mode=(stat.S_IFLNK if symlink else stat.S_IFDIR) | 0o1777)))
                stack.enter_context(patch('os.statvfs', return_value=SimpleNamespace(
                    f_bsize=4096, f_frsize=4096, f_namemax=255)))
                mkdir = stack.enter_context(patch('os.mkdir', side_effect=None if missing else FileExistsError))
                umask = stack.enter_context(patch('os.umask', return_value=0o022))
                chmod = stack.enter_context(patch('os.chmod'))
                set_hostname = stack.enter_context(patch('socket.sethostname', create=True))
                stack.enter_context(patch('socket.gethostname', return_value='sandproof-paired'))
                output = stack.enter_context(redirect_stdout(io.StringIO()))
                if mount == 'tmpfs' or symlink:
                    with self.assertRaises(AssertionError):
                        exec(compile(command[3], '<native-environment-fixture>', 'exec'), {})
                    set_hostname.assert_not_called()
                else:
                    exec(compile(command[3], '<native-environment-fixture>', 'exec'), {})
                    self.assertEqual(json.loads(output.getvalue()), environment_receipt())
                    if initialize:
                        set_hostname.assert_called_once_with(b'sandproof-paired')
                    else:
                        set_hostname.assert_not_called()
                if initialize:
                    mkdir.assert_called_once_with('/var/tmp', 0o1777)
                    self.assertEqual(umask.call_args_list, [unittest.mock.call(0), unittest.mock.call(0o022)])
                else:
                    mkdir.assert_not_called()
                    umask.assert_not_called()
                chmod.assert_not_called()


class OriginalSdkRecordingTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.app = SyntheticEnvd()
        self.server = make_server('127.0.0.1', 0, self.app,
                                 server_class=ThreadedEnvd, handler_class=QuietHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        self.assertFalse(self.thread.is_alive())

    async def connect(self, cls):
        url = f'http://127.0.0.1:{self.server.server_port}'
        return await probe.call(cls.connect, 'probe-sandbox', api_url=url, sandbox_url=url,
            api_key='synthetic-only', domain='example.invalid', retries=0, request_timeout=15)

    async def test_both_original_sdks_record_callbacks_nonzero_errors_and_exact_binary_streams(self):
        observed = []
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            calls = probe.Calls(sandbox, cls is AsyncSandbox)
            self.app.process.exit_code = 0
            self.app.process.stdout_chunks = ['a中'.encode(), b'b']
            self.app.process.stderr_chunks = [b'err']
            result = await calls.invoke('commands.run', 'shared', cwd='/tmp/shared',
                                        envs={'VALUE': 'literal'}, user='ubuntu', timeout=15)
            self.assertEqual((result.stdout, result.stderr), ('a中b', 'err'))
            self.app.process.exit_code = 17
            await calls.invoke('commands.run', 'exit 17', user='ubuntu', timeout=15,
                               expected=CommandExitException)
            self.app.file_contents['/tmp/shared/data'] = bytes(range(256)) * 129
            data = await calls.stream('/tmp/shared/data', 'ubuntu')
            self.assertEqual(data, self.app.file_contents['/tmp/shared/data'])
            self.app.file_status = 404
            await calls.invoke('files.read', '/tmp/shared/missing', user='ubuntu',
                               request_timeout=15, expected=FileNotFoundException)
            self.app.file_status = 200
            observed.append(calls.rows)
        probe.compare(*observed, 'sync/async actual SDK')
        self.assertEqual(len(self.app.process.starts), 4)
        for request in self.app.process.starts[::2]:
            self.assertEqual((request.process.cmd, list(request.process.args)),
                             ('/bin/bash', ['-l', '-c', 'shared']))
            self.assertEqual(dict(request.process.envs), {'VALUE': 'literal'})
        self.assertEqual(observed[0][1]['error']['fields']['exit_code'], 17)
        self.assertIn('synthetic rejection', observed[0][3]['error']['message'])

    async def test_stream_truncation_is_not_a_successful_record_or_retried_operation(self):
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            calls = probe.Calls(sandbox, cls is AsyncSandbox)
            self.app.truncate_file = True
            before = len(self.app.records)
            with self.assertRaises(Exception):
                await calls.stream('/tmp/truncated', 'root')
            self.assertEqual(len(self.app.records), before + 1)
            self.assertEqual(len(calls.rows), 1)
            self.assertEqual(calls.rows[0]['status'], 'incomplete')
            self.assertNotIn('result', calls.rows[0])
            self.app.truncate_file = False
            self.assertEqual(await calls.stream('/tmp/healthy', 'root'), b'synthetic file content\n')

    async def test_environment_receipt_both_sdks_retains_raw_failure_and_never_replays_setup(self):
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            for fault in (None, 'filesystem', 'hostname', 'boolean-block-size'):
                receipt = environment_receipt()
                if fault == 'filesystem':
                    receipt['filesystem'] = 'tmpfs'
                elif fault == 'hostname':
                    receipt['hostname'] = 'other-host'
                elif fault == 'boolean-block-size':
                    receipt['block_size'] = True
                raw = json.dumps(receipt)
                self.app.process.stdout_chunks = [raw.encode()]
                self.app.process.stderr_chunks = []
                before = len(self.app.process.starts)
                evidence = {}
                with self.subTest(sdk=cls.__name__, fault=fault):
                    if fault is None:
                        self.assertEqual(await probe.environment(sandbox, evidence, initialize=True), receipt)
                        self.assertEqual(evidence['status'], 'complete')
                    else:
                        with self.assertRaises(AssertionError):
                            await probe.environment(sandbox, evidence, initialize=True)
                        self.assertEqual(evidence['status'], 'incomplete')
                        self.assertNotIn('receipt', evidence)
                    self.assertEqual(evidence['result']['stdout'], raw)
                    self.assertEqual(len(self.app.process.starts), before + 1)
                    start = self.app.process.starts[-1]
                    self.assertEqual(list(start.process.args), ['-l', '-c', probe.environment_command(initialize=True)])
                    self.assertEqual(self.app.process.signals, [])

    async def test_native_environment_failure_keeps_original_exception_and_bytes_without_retry(self):
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            self.app.process.stdout_chunks = [b'original partial receipt']
            self.app.process.stderr_chunks = [b'original native setup failure']
            self.app.process.exit_code = 1
            before = len(self.app.process.starts)
            evidence = {}
            with self.assertRaises(CommandExitException):
                await probe.environment(sandbox, evidence, initialize=True)
            self.assertEqual(evidence['status'], 'incomplete')
            self.assertNotIn('receipt', evidence)
            self.assertEqual(evidence['result']['exit_code'], 1)
            self.assertEqual(evidence['result']['stdout'], 'original partial receipt')
            self.assertEqual(evidence['result']['stderr'], 'original native setup failure')
            self.assertEqual(len(self.app.process.starts), before + 1)
            self.assertEqual(self.app.process.signals, [])

    async def test_resource_samples_use_both_original_sdks_once_per_phase_without_budget_verdict(self):
        expected = {'fixed_go': guest_probe.load_go_expected()['file']}
        daemon = dict(pid=42, start_ticks=123, socket_inode=77)
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            samples = resources.ResourceSamples(sandbox, expected, daemon, probe.plan())
            before = len(self.app.process.starts)
            self.app.process.stderr_chunks = []
            for index, phase in enumerate(samples.phases):
                # Large valid readings are retained without a made-up pass
                # threshold; only this collector's representation is bounded.
                report = resource_report(expected, daemon, rss=2**40 + index)
                raw = json.dumps(report).encode()
                self.app.process.stdout_chunks = [raw[:17], raw[17:]]
                await samples.take(phase)
                self.assertEqual(samples.report['observations'][-1]['receipt'], report)
                self.assertGreaterEqual(samples.report['observations'][-1]['request_elapsed_ns'], 0)
            samples.complete()
            self.assertEqual(len(samples.phases), 14)
            self.assertEqual(samples.report['budget_verdict'], 'not_evaluated')
            self.assertEqual(len(self.app.process.starts) - before, 14)
            for start in self.app.process.starts[before:]:
                self.assertEqual((start.process.cmd, list(start.process.args)),
                                 ('/bin/bash', ['-l', '-c', samples.command]))
            with self.assertRaises(AssertionError):
                await samples.take('after-cases')
            self.assertEqual(len(self.app.process.starts) - before, 14)
            self.assertEqual(self.app.process.signals, [])

    async def test_resource_reply_overflow_stderr_wrong_identity_or_missing_end_never_retries(self):
        expected = {'fixed_go': guest_probe.load_go_expected()['file']}
        daemon = dict(pid=42, start_ticks=123, socket_inode=77)
        for cls in (Sandbox, AsyncSandbox):
            sandbox = await self.connect(cls)
            for fault in ('overflow', 'stderr', 'identity', 'missing-end'):
                with self.subTest(sdk=cls.__name__, fault=fault):
                    samples = resources.ResourceSamples(sandbox, expected, daemon, probe.plan())
                    report = resource_report(expected, dict(daemon, pid=43) if fault == 'identity' else daemon)
                    self.app.process.stdout_chunks = ([b'x' * 4097] if fault == 'overflow'
                                                       else [json.dumps(report).encode()])
                    self.app.process.stderr_chunks = [b'unexpected'] if fault == 'stderr' else []
                    self.app.process.omit_end = fault == 'missing-end'
                    before = len(self.app.process.starts)
                    with self.assertRaises(Exception):
                        await samples.take('before-cases')
                    self.assertEqual(samples.report['observations'][0]['status'], 'incomplete')
                    self.assertNotIn('receipt', samples.report['observations'][0])
                    for phase in ('before-cases', samples.phases[1]):
                        with self.assertRaises(AssertionError):
                            await samples.take(phase)
                    with self.assertRaises(AssertionError):
                        samples.complete()
                    self.assertEqual(len(self.app.process.starts) - before, 1)
                    self.assertEqual(self.app.process.signals, [])
                    self.app.process.omit_end = False


class ResourceConcurrencyTests(unittest.IsolatedAsyncioTestCase):
    async def test_pending_sample_rejects_concurrent_work_and_cancellation_retains_one_failed_phase(self):
        expected = {'fixed_go': guest_probe.load_go_expected()['file']}
        daemon = dict(pid=42, start_ticks=123, socket_inode=77)
        entered, release = asyncio.Event(), asyncio.Event()
        async def run(*args, **kwargs):
            entered.set()
            await release.wait()
            raise AssertionError('cancelled observation must not finish')
        method = AsyncMock(side_effect=run)
        samples = resources.ResourceSamples(SimpleNamespace(commands=SimpleNamespace(run=method)),
                                             expected, daemon, probe.plan())
        with self.assertRaises(AssertionError):
            await samples.take('after-cases')
        method.assert_not_called()
        with self.assertRaises(AssertionError):
            samples.complete()
        task = asyncio.create_task(samples.take('before-cases'))
        try:
            await asyncio.wait_for(entered.wait(), 5)
            with self.assertRaises(AssertionError):
                await samples.take(samples.phases[1])
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
        finally:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        self.assertEqual(len(samples.report['observations']), 1)
        row = samples.report['observations'][0]
        self.assertGreaterEqual(row['request_elapsed_ns'], 0)
        self.assertEqual({key: value for key, value in row.items() if key != 'request_elapsed_ns'},
                         dict(phase='before-cases', status='incomplete', error='CancelledError'))
        with self.assertRaises(AssertionError):
            await samples.take('before-cases')
        method.assert_awaited_once()


class PairedOrchestrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(isolated_sdk_environment())
        self.stack.enter_context(patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'synthetic-only'}))
        self.args = SimpleNamespace(api_url='http://127.0.0.1:1', sandbox_url='http://127.0.0.1:1',
            go_template='pinned-go', template='pinned-rust', guest_release='/private/release',
            guest_manifest_sha256='a'*64, guest_configuration='/private/config',
            guest_configuration_sha256='b'*64)
        expected = {role: {'bytes': 7, 'sha256': 'c'*64} for role in guest_probe.NAMES}
        self.expected = expected
        self.replace(guest_probe, 'load_expected', Mock(return_value=({'source_commit': 'd'*40}, expected)))
        self.go = guest_probe.load_go_expected()
        self.go_report = dict(schema_version=1, fixed_go=self.go['file'],
                              daemon=dict(pid=42, start_ticks=123, socket_inode=77))
        self.guests = []
        for index in range(4):
            wrap = AsyncMock if index >= 2 else Mock
            self.guests.append(SimpleNamespace(sandbox_id=f'sandbox-{index+1}',
                commands=SimpleNamespace(run=wrap(return_value=SimpleNamespace(
                    exit_code=0, stdout=json.dumps(self.go_report), stderr=''))),
                kill=wrap(return_value=True)))
        self.sync_create = self.replace(probe.Sandbox, 'create', Mock(side_effect=self.guests[:2]))
        self.async_create = self.replace(probe.AsyncSandbox, 'create', AsyncMock(side_effect=self.guests[2:]))
        self.observe = self.replace(probe, 'observe_candidate', AsyncMock(return_value={'pinned': 'rust'}))
        async def environment(_sandbox, evidence, *, initialize):
            evidence.update(status='complete', receipt=environment_receipt())
            return environment_receipt()
        self.environment = self.replace(probe, 'environment', AsyncMock(side_effect=environment))
        self.run_cases = self.replace(probe, 'cases', AsyncMock(side_effect=lambda *_args: rows()))
        async def file_cases(_sandbox, _inputs, asynchronous, partial):
            partial.append(dict(method='files.mode', args=[asynchronous], status='complete'))
        self.run_files = self.replace(probe, 'sdk_file_cases', AsyncMock(side_effect=file_cases))
        async def policy_cases(_sandbox, _inputs, _asynchronous, kind, partial):
            partial.append(dict(kind=kind, status='complete'))
        self.run_policy = self.replace(probe, 'policy_cases', AsyncMock(side_effect=policy_cases))

    def replace(self, target, name, replacement):
        return self.stack.enter_context(patch.object(target, name, replacement))

    async def run_probe(self, error=None):
        output = io.StringIO()
        with redirect_stdout(output):
            if error is None:
                await probe.exercise(self.args)
            else:
                with self.assertRaises(error) as raised:
                    await probe.exercise(self.args)
                self.failure = raised.exception
        return json.loads(output.getvalue()) if output.getvalue() else None

    async def test_four_known_guests_share_one_plan_and_get_before_after_identity_and_cleanup(self):
        result = await self.run_probe()
        self.assertEqual(result['status'], 'pass')
        self.assertEqual(self.run_cases.await_count, 4)
        plans = [call.args[1] for call in self.run_cases.await_args_list]
        self.assertTrue(all(item is plans[0] for item in plans))
        self.assertEqual(self.run_files.await_count, 4)
        self.assertTrue(all(call.args[1] is plans[0] for call in self.run_files.await_args_list))
        self.assertEqual([call.args[2] for call in self.run_files.await_args_list], [False, False, True, True])
        self.assertEqual(self.run_policy.await_count, 4)
        self.assertTrue(all(call.args[1] is plans[0] for call in self.run_policy.await_args_list))
        self.assertEqual([(call.args[2], call.args[3]) for call in self.run_policy.await_args_list],
                         [(False, 'go'), (False, 'rust'), (True, 'go'), (True, 'rust')])
        self.assertEqual(plans[0], result['inputs'])
        for create in (self.sync_create, self.async_create):
            self.assertEqual([call.kwargs['template'] for call in create.call_args_list],
                             ['pinned-go', 'pinned-rust'])
            for call in create.call_args_list:
                self.assertEqual(call.kwargs['retries'], 0)
                self.assertFalse(call.kwargs['allow_internet_access'])
        for index, guest in enumerate(self.guests):
            guest.kill.assert_called_once_with()
            self.assertEqual(guest.commands.run.call_count, 2 if index % 2 == 0 else 0)
        self.assertEqual(self.observe.await_count, 4)
        self.assertEqual([item.kwargs['initialize'] for item in self.environment.await_args_list],
                         [True, False] * 4)
        for item in result['results']:
            self.assertTrue(item['known_destroy_acknowledged'])
            self.assertEqual(len(item['pair']), 2)
            self.assertTrue(all('resources' not in entry for entry in item['pair']))
            for entry in item['pair']:
                self.assertEqual(entry['tag_policy'], [dict(kind=entry['kind'], status='complete')])
                self.assertEqual(entry['environment_before'], entry['environment_after'])
        self.assertNotIn('resource_sampling', result)

    async def assert_environment_fault(self, *, after_change):
        async def environment(sandbox, evidence, *, initialize):
            receipt = environment_receipt()
            if (after_change and not initialize) or (not after_change and sandbox is self.guests[1]):
                receipt['block_size'] = 1024
            evidence.update(status='complete', receipt=receipt)
            return receipt
        self.environment.side_effect = environment
        result = await self.run_probe(AssertionError)
        pair = result['results'][0]['pair']
        self.assertEqual(len(pair), 1 if after_change else 2)
        self.assertEqual(self.run_cases.await_count, 1)
        self.assertEqual(self.environment.await_count, 2 if after_change else 3)
        self.assertTrue(result['results'][0]['known_destroy_acknowledged'])
        self.async_create.assert_not_called()

    async def test_actual_filesystem_shape_mismatch_stops_before_second_side_cases(self):
        await self.assert_environment_fault(after_change=False)

    async def test_environment_change_after_cases_fails_without_repair_or_replay(self):
        await self.assert_environment_fault(after_change=True)

    async def test_optional_resource_plan_samples_each_pinned_vm_and_preserves_protocol_comparison(self):
        self.args.observe_resources = True
        daemon = dict(pid=52, start_ticks=345, socket_inode=88)
        self.observe.return_value = {'observation': {'daemon': daemon}}
        for index, guest in enumerate(self.guests):
            expected = self.expected if index % 2 else {'fixed_go': self.go['file']}
            original = daemon if index % 2 else self.go_report['daemon']
            def run(command, *, _expected=expected, _daemon=original, **kwargs):
                if 'on_stdout' not in kwargs:
                    return SimpleNamespace(exit_code=0, stdout=json.dumps(self.go_report), stderr='')
                self.assertEqual(command, guest_probe.resource_command(_expected, _daemon))
                self.assertEqual((kwargs['user'], kwargs['timeout'], kwargs['request_timeout']), ('root', 20, 20))
                raw = json.dumps(resource_report(_expected, _daemon))
                kwargs['on_stdout'](raw[:13])
                kwargs['on_stdout'](raw[13:])
                kwargs['on_stderr']('')
                return SimpleNamespace(exit_code=0, stdout=raw, stderr='')
            guest.commands.run.side_effect = run
        async def cases(_sandbox, inputs, _asynchronous, partial, *, resource_sample):
            for phase in resources.phases(inputs)[1:-1]:
                await resource_sample(phase)
            partial.extend(rows())
            return partial
        self.run_cases.side_effect = cases
        result = await self.run_probe()
        self.assertEqual(result['status'], 'pass')
        self.assertIn('no resource budget verdict', result['resource_sampling'])
        for item in result['results']:
            for entry in item['pair']:
                samples = entry['resources']
                self.assertEqual(samples['budget_verdict'], 'not_evaluated')
                self.assertEqual([row['phase'] for row in samples['observations']], resources.phases(result['inputs']))
                self.assertEqual(entry['calls'], rows())
        for index, guest in enumerate(self.guests):
            guest.kill.assert_called_once()
            self.assertEqual(guest.commands.run.call_count, 14 + (2 if index % 2 == 0 else 0))

    async def test_failed_resource_receipt_keeps_partial_phase_and_known_vm_cleanup_failure(self):
        self.args.observe_resources = True
        def run(command, **kwargs):
            if 'on_stdout' not in kwargs:
                return SimpleNamespace(exit_code=0, stdout=json.dumps(self.go_report), stderr='')
            raw = json.dumps(resource_report({'fixed_go': self.go['file']},
                                             dict(self.go_report['daemon'], pid=43)))
            kwargs['on_stdout'](raw)
            return SimpleNamespace(exit_code=0, stdout=raw, stderr='')
        self.guests[0].commands.run.side_effect = run
        self.guests[0].kill.side_effect = RuntimeError('known VM cleanup uncertain')
        result = await self.run_probe(BaseExceptionGroup)
        row = result['results'][0]['pair'][0]['resources']['observations'][0]
        self.assertGreaterEqual(row['request_elapsed_ns'], 0)
        self.assertEqual({key: value for key, value in row.items() if key != 'request_elapsed_ns'},
                         dict(phase='before-cases', status='incomplete', error='ValueError'))
        self.assertIsInstance(self.failure.exceptions[0], ValueError)
        self.assertFalse(result['results'][0]['known_destroy_acknowledged'])
        self.run_cases.assert_not_called()
        self.sync_create.assert_called_once()
        self.guests[0].kill.assert_called_once()
        self.assertEqual(self.guests[0].commands.run.call_count, 2)

    async def test_comparison_failure_preserves_both_observations_and_all_cleanup_errors(self):
        different = rows()
        different[3]['error']['message'] = 'wrong error'
        self.run_cases.side_effect = [rows(), different]
        self.guests[1].kill.side_effect = RuntimeError('cleanup uncertain')
        result = await self.run_probe(BaseExceptionGroup)
        self.assertEqual(result['status'], 'failed')
        pair = result['results'][0]['pair']
        self.assertEqual(pair[1]['calls'][3]['error']['message'], 'wrong error')
        self.assertEqual(pair[0]['calls'], rows())
        self.assertFalse(result['results'][0]['known_destroy_acknowledged'])
        self.assertIsInstance(self.failure.exceptions[0], AssertionError)
        for guest in self.guests[:2]:
            guest.kill.assert_called_once_with()
        self.async_create.assert_not_called()

    async def test_bad_go_or_changed_rust_identity_cannot_count_as_a_pair(self):
        self.guests[0].commands.run.return_value.stdout = '{}'
        result = await self.run_probe(ValueError)
        self.assertEqual(result['status'], 'failed')
        self.run_cases.assert_not_called()
        self.assertEqual(self.sync_create.call_count, 1)
        self.guests[0].kill.assert_called_once()

    async def test_policy_failure_retains_partial_outcomes_and_cleans_every_known_vm(self):
        async def policy(_sandbox, _inputs, _asynchronous, kind, partial):
            partial.append(dict(kind=kind, status='incomplete'))
            raise AssertionError('unexpected tag outcome')
        self.run_policy.side_effect = policy
        result = await self.run_probe(AssertionError)
        entry = result['results'][0]['pair'][0]
        self.assertEqual(entry['tag_policy'], [dict(kind='go', status='incomplete')])
        self.assertNotIn('after', entry)
        self.guests[0].kill.assert_called_once()
        self.sync_create.assert_called_once()
        self.async_create.assert_not_called()

    async def test_unknown_second_create_does_not_retry_or_invent_a_vm_to_destroy(self):
        self.sync_create.side_effect = [self.guests[0], RuntimeError('unknown create')]
        result = await self.run_probe(RuntimeError)
        self.assertEqual(result['status'], 'failed')
        self.assertEqual(self.sync_create.call_count, 2)
        self.assertEqual(len(result['results'][0]['pair']), 1)
        self.assertEqual(result['results'][0]['create_attempts'][1],
                         dict(kind='rust', template='pinned-rust', outcome='unknown'))
        self.guests[0].kill.assert_called_once()
        for guest in self.guests[1:]:
            guest.kill.assert_not_called()

    async def test_incomplete_sdk_call_stays_in_failed_report_without_an_after_receipt(self):
        async def fail_case(_sandbox, _inputs, _asynchronous, partial):
            partial.append({'method': 'commands.run', 'args': ['unknown launch'],
                            'kwargs': {}, 'status': 'incomplete'})
            raise TimeoutError('original Start outcome unknown')
        self.run_cases.side_effect = fail_case
        result = await self.run_probe(TimeoutError)
        entry = result['results'][0]['pair'][0]
        self.assertEqual(entry['calls'][0]['status'], 'incomplete')
        self.assertNotIn('after', entry)
        self.assertEqual(self.guests[0].commands.run.call_count, 1)
        self.assertEqual(self.sync_create.call_count, 1)
        self.guests[0].kill.assert_called_once()

    async def test_changed_rust_process_is_recorded_and_rejected_before_next_sdk_mode(self):
        self.observe.side_effect = [{'pid': 42}, {'pid': 43}]
        result = await self.run_probe(AssertionError)
        self.assertEqual(result['status'], 'failed')
        rust = result['results'][0]['pair'][1]
        self.assertNotEqual(rust['before'], rust['after'])
        self.async_create.assert_not_called()
        for guest in self.guests[:2]:
            guest.kill.assert_called_once()

    async def test_sdk_specific_batch_mismatch_is_rejected_and_partial_failure_is_retained(self):
        async def mismatch(sandbox, _inputs, _asynchronous, partial):
            partial.append(dict(method='files.mode', args=[sandbox.sandbox_id], status='complete'))
        self.run_files.side_effect = mismatch
        result = await self.run_probe(AssertionError)
        self.assertIn('SDK-specific octet', str(self.failure))
        self.assertEqual(len(result['results'][0]['pair']), 2)
        self.async_create.assert_not_called()
        for guest in self.guests[:2]:
            guest.kill.assert_called_once()

    async def test_incomplete_batch_observations_survive_without_repeating_uploads(self):
        async def incomplete(_sandbox, _inputs, _asynchronous, partial):
            partial.append(dict(method='files.octet_batch', status='incomplete',
                                settlement={'status': 'incomplete', 'calls': ['partial evidence']}))
            raise TimeoutError('batch observation incomplete')
        self.run_files.side_effect = incomplete
        result = await self.run_probe(TimeoutError)
        entry = result['results'][0]['pair'][0]
        self.assertEqual(entry['sdk_files'][0]['settlement']['calls'], ['partial evidence'])
        self.assertNotIn('after', entry)
        self.run_files.assert_awaited_once()
        self.sync_create.assert_called_once()
        self.guests[0].kill.assert_called_once()

    async def test_missing_rust_pin_or_same_template_rejects_before_create(self):
        self.args.guest_manifest_sha256 = None
        self.assertIsNone(await self.run_probe(ValueError))
        self.args.guest_manifest_sha256 = 'a'*64
        self.args.go_template = self.args.template
        self.assertIsNone(await self.run_probe(AssertionError))
        self.sync_create.assert_not_called()
        self.async_create.assert_not_called()


if __name__ == '__main__':
    unittest.main()
