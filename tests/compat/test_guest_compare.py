"""Paired probe comparison/orchestration and actual fixed SDK loopback calls."""
import copy
import argparse
from contextlib import ExitStack, redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
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


def rows():
    return [{'method': 'commands.run', 'args': ['fixed input'], 'kwargs': {'user': 'root'},
             'result': {'stdout': 'a中b', 'stderr': '', 'exit_code': 0, 'error': None},
             'callbacks': {'stdout': ['a', '中b'], 'stderr': []}},
            {'method': 'files.list', 'args': ['/tmp/shared'], 'kwargs': {'depth': 4},
             'result': [{'path': '/tmp/shared/a', 'mode': 1, 'modified_time': 'fixed'},
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
        self.run_cases = self.replace(probe, 'cases', AsyncMock(side_effect=lambda *_args: rows()))

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
        for item in result['results']:
            self.assertTrue(item['known_destroy_acknowledged'])
            self.assertEqual(len(item['pair']), 2)

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
