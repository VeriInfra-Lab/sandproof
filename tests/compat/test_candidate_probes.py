"""Existing SDK system probes with controlled calls; no network, VM or canary."""
import argparse
import asyncio
from contextlib import ExitStack, redirect_stdout
import copy
import io
import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

from tools import guest_probe

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_command_arguments as arguments
import sdk_command_input as inputs
import sdk_data_contract as data
import sdk_file_permissions as files
import sdk_command_timeouts as command_timeouts
import sdk_file_timeouts as file_timeouts
import sdk_rpc_timeouts as rpc_timeouts
import sdk_pty as pty_probe

TIMEOUT_PROBES = (command_timeouts, file_timeouts, rpc_timeouts)
PROBES = (inputs, arguments, files, pty_probe, *TIMEOUT_PROBES)


class CandidateProbeTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.expected = {role: dict(bytes=7, sha256='c'*64) for role in guest_probe.NAMES}
        self.identity = {'source_commit': 'd'*40}
        self.args = SimpleNamespace(api_url='http://127.0.0.1:1', sandbox_url='http://127.0.0.1:1',
            template='selected-rust-candidate', guest_release='/private/release',
            guest_manifest_sha256='a'*64, guest_configuration='/private/config',
            guest_configuration_sha256='b'*64, host_canary='/unused/host-canary')
        self.stack.enter_context(patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'synthetic-owner'}))
        self.stack.enter_context(patch.object(guest_probe, 'load_expected', side_effect=self.load))
        # These are the same pinned SDK class objects imported by all probes.
        self.stack.enter_context(patch.object(inputs.Sandbox, 'create', side_effect=self.create_sync))
        self.stack.enter_context(patch.object(inputs.AsyncSandbox, 'create', new=AsyncMock(side_effect=self.create_async)))
        for module, name, label in [(inputs, 'user_cases', 'stdin'),
                                    (pty_probe, 'user_cases', 'pty'),
                                    (arguments, 'argument_cases', 'arguments'),
                                    (files, 'user_cases', 'users'),
                                    (files, 'path_cases', 'paths'),
                                    (files, 'batch_cases', 'batch'),
                                    (files, 'isolation_cases', 'isolation')]:
            self.stack.enter_context(patch.object(module, name, new=AsyncMock(side_effect=self.case(label))))
        for module, names in [(command_timeouts, ('run_case', 'connect_case', 'wait_deadline_case')),
                              (file_timeouts, ('read_case', 'write_case')),
                              (rpc_timeouts, ('file_case', 'command_case'))]:
            for name in names:
                self.stack.enter_context(patch.object(module, name, new=AsyncMock(side_effect=self.timeout_case(name))))
        self.reset()

    def reset(self):
        self.trace, self.created = [], []
        self.count = {'sync': 0, 'async': 0}
        self.bad_observation = self.changed_observation = self.fail_create = self.fail_case = None
        self.kill_failures = {}

    def load(self, *values):
        self.trace.append(('material', values))
        return self.identity, self.expected

    def create_sync(self, **kwargs):
        return self.create('sync', kwargs)

    async def create_async(self, **kwargs):
        return self.create('async', kwargs)

    def create(self, mode, kwargs):
        self.count[mode] += 1
        number = self.count[mode]
        self.trace.append(('create', mode, number, kwargs))
        if self.fail_create == (mode, number):
            raise RuntimeError('controlled create outcome unknown')
        ident = f'{mode}-{number}'
        observations = 0
        def run(command, **options):
            nonlocal observations
            if command.startswith('/usr/bin/python3 -I -c '):
                observations += 1
                self.trace.append(('observe', ident, observations, options))
                report = dict(schema_version=1, files=copy.deepcopy(self.expected),
                              daemon=dict(pid=42, start_ticks=123, socket_inode=77))
                if self.bad_observation == (mode, number):
                    report['files']['service']['sha256'] = '0' * 64
                if self.changed_observation == (mode, number) and observations == 2:
                    report['daemon']['start_ticks'] += 1
                return SimpleNamespace(exit_code=0, stdout=json.dumps(report), stderr='')
            self.trace.append(('command', ident, command))
            return SimpleNamespace(exit_code=0, stdout={'printf stdin-healthy': 'stdin-healthy',
                'printf healthy': 'healthy'}.get(command, ''), stderr='')
        def write(path, content):
            self.trace.append(('setup_file', ident, path, content))
        def kill():
            self.trace.append(('kill', ident))
            failure = self.kill_failures.get(ident)
            if isinstance(failure, BaseException):
                raise failure
            return failure if failure is not None else True
        handle = SimpleNamespace(sandbox_id=ident,
            commands=SimpleNamespace(run=AsyncMock(side_effect=run) if mode == 'async' else Mock(side_effect=run)),
            files=SimpleNamespace(write=AsyncMock(side_effect=write) if mode == 'async' else Mock(side_effect=write)),
            kill=AsyncMock(side_effect=kill) if mode == 'async' else Mock(side_effect=kill))
        self.created.append(handle)
        return handle

    def case(self, label):
        async def run(sandbox, *args):
            self.trace.append(('case', sandbox.sandbox_id, label))
            if self.fail_case == (sandbox.sandbox_id, label):
                raise ValueError('controlled case failure')
            return [dict(case=label, user=args[0] if label == 'stdin' else 'root')]
        return run

    def timeout_case(self, label):
        async def run(sandbox, *args):
            self.trace.append(('case', sandbox.sandbox_id, label, args))
            if self.fail_case == (sandbox.sandbox_id, label):
                raise ValueError('controlled case failure')
            return dict(case=label)
        return run

    async def execute(self, module):
        output = io.StringIO()
        with redirect_stdout(output):
            await module.exercise(self.args)
        return json.loads(output.getvalue())

    def assert_cleaned_once(self):
        killed = [item[1] for item in self.trace if item[0] == 'kill']
        self.assertCountEqual(killed, [item.sandbox_id for item in self.created])
        self.assertEqual(len(killed), len(set(killed)))

    async def test_each_probe_selects_same_candidate_and_brackets_both_sdk_modes(self):
        for module in PROBES:
            with self.subTest(probe=module.__name__):
                self.reset()
                report = await self.execute(module)
                self.assertEqual(report['template'], self.args.template)
                self.assertEqual(self.trace[0][0], 'material')
                self.assertEqual(len(self.created), 4 if module is files else 2)
                for event in self.trace:
                    if event[0] == 'create':
                        self.assertEqual(event[3]['template'], self.args.template)
                        self.assertFalse(event[3]['allow_internet_access'])
                        self.assertEqual(event[3]['retries'], 0)
                for sandbox in self.created:
                    events = [event for event in self.trace if len(event) > 1 and event[1] == sandbox.sandbox_id]
                    self.assertEqual([event[2] for event in events if event[0] == 'observe'], [1, 2])
                    self.assertEqual(events[0][0], 'observe')
                    self.assertEqual([event[0] for event in events][-2:], ['observe', 'kill'])
                if module is inputs:
                    receipts = report['guests']
                elif module is not files:
                    receipts = report['results']
                else:
                    receipts = [guest for item in report['results'] for guest in item['rust_guests']]
                self.assertCountEqual([item['sandbox_id'] for item in receipts],
                                      [item.sandbox_id for item in self.created])
                for item in receipts:
                    self.assertEqual(item['rust_guest']['identity'], self.identity)
                self.assert_cleaned_once()

    async def test_missing_pin_is_rejected_before_any_probe_creates_a_vm(self):
        self.args.guest_manifest_sha256 = None
        for module in PROBES:
            with self.subTest(probe=module.__name__):
                self.reset()
                with self.assertRaises(ValueError):
                    await self.execute(module)
                self.assertEqual(self.trace, [])

    async def test_bad_initial_observation_prevents_cases_and_cleans_every_known_instance(self):
        for module in PROBES:
            with self.subTest(probe=module.__name__):
                self.reset()
                self.bad_observation = ('sync', 1)
                with self.assertRaises(ValueError):
                    await self.execute(module)
                self.assertFalse(any(event[0] in ('case', 'command', 'setup_file') for event in self.trace))
                self.assertEqual(self.count['async'], 0)
                self.assert_cleaned_once()

    async def test_changed_process_after_cases_is_not_accepted_for_either_sdk_mode(self):
        for module in PROBES:
            for mode in ('sync', 'async'):
                with self.subTest(probe=module.__name__, mode=mode):
                    self.reset()
                    self.changed_observation = (mode, 2 if module is files else 1)
                    with self.assertRaisesRegex(AssertionError, 'identity changed'):
                        await self.execute(module)
                    self.assertTrue(any(event[0] == 'case' and event[1].startswith(mode) for event in self.trace))
                    self.assert_cleaned_once()

    async def test_file_cleanup_attempts_both_original_instances_and_keeps_primary_failure(self):
        for mode in ('sync', 'async'):
            with self.subTest(mode=mode):
                self.reset()
                self.fail_case = (mode + '-1', 'batch')
                self.kill_failures = {mode + '-2': False, mode + '-1': OSError('controlled cleanup failure')}
                with self.assertRaises(BaseExceptionGroup) as caught:
                    await self.execute(files)
                group = caught.exception
                self.assertIsInstance(group.exceptions[0], ValueError)
                self.assertEqual(str(group.exceptions[0]), 'controlled case failure')
                self.assertEqual([type(error) for error in group.exceptions[1].exceptions],
                                 [RuntimeError, OSError])
                self.assertEqual([event[1] for event in self.trace if event[0] == 'kill'][-2:],
                                 [mode + '-2', mode + '-1'])
                self.assert_cleaned_once()

    async def test_file_failure_report_preserves_modes_and_unknown_create_without_replay(self):
        for mode in ('sync', 'async'):
            for number in (1, 2):
                with self.subTest(mode=mode, number=number):
                    self.reset()
                    self.fail_create = (mode, number)
                    output = io.StringIO()
                    with redirect_stdout(output), self.assertRaisesRegex(RuntimeError, 'create outcome unknown'):
                        await files.exercise(self.args)
                    report = json.loads(output.getvalue())
                    self.assertEqual(report['status'], 'failed')
                    self.assertEqual(report['failure'], 'RuntimeError')
                    self.assertEqual(len(report['results']), 1 if mode == 'sync' else 2)
                    if mode == 'async':
                        self.assertEqual(report['results'][0]['status'], 'pass')
                        self.assertEqual(len(report['results'][0]['cases']), 4)
                    failed = report['results'][-1]
                    self.assertEqual(failed['status'], 'failed')
                    self.assertEqual(failed['phase'], 'create')
                    self.assertEqual(failed['cases'], [])
                    self.assertEqual(len(failed['create_attempts']), number)
                    self.assertEqual(failed['create_attempts'][-1], dict(ordinal=number, outcome='unknown'))
                    self.assertTrue(failed['known_destroy_acknowledged'])
                    self.assertEqual(self.count[mode], number)
                    self.assert_cleaned_once()
                    self.assertNotIn('synthetic-owner', output.getvalue())

    async def test_file_failure_report_keeps_completed_groups_and_cleanup_errors(self):
        self.fail_case = ('sync-1', 'batch')
        self.kill_failures['sync-2'] = False
        output = io.StringIO()
        with redirect_stdout(output), self.assertRaises(BaseExceptionGroup) as caught:
            await files.exercise(self.args)
        report = json.loads(output.getvalue())
        failed = report['results'][0]
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(failed['phase'], 'batch')
        self.assertEqual(failed['failure'], 'ValueError')
        self.assertEqual([item['case'] for item in failed['cases']], ['users', 'paths'])
        self.assertFalse(failed['known_destroy_acknowledged'])
        self.assertEqual(failed['cleanup_failure'], 'ExceptionGroup')
        self.assertIsInstance(caught.exception.exceptions[0], ValueError)
        self.assertEqual(self.count['async'], 0)
        self.assert_cleaned_once()

    async def test_second_create_failure_only_cleans_known_instance_without_replay(self):
        for mode in ('sync', 'async'):
            with self.subTest(mode=mode):
                self.reset()
                self.fail_create = (mode, 2)
                with self.assertRaisesRegex(RuntimeError, 'create outcome unknown'):
                    await self.execute(files)
                self.assertEqual(self.count[mode], 2)
                self.assert_cleaned_once()

    async def test_original_default_reports_and_base_selection_do_not_require_guest_material(self):
        for name in ('template', 'guest_release', 'guest_manifest_sha256', 'guest_configuration',
                     'guest_configuration_sha256'):
            setattr(self.args, name, None)
        for module in PROBES:
            with self.subTest(probe=module.__name__):
                self.reset()
                report = await self.execute(module)
                expected = {'sdk_version', 'scope', 'cases' if module is inputs else 'results'}
                if module is files:
                    expected.add('status')
                if module is file_timeouts:
                    expected.update(('payload_bytes', 'sha256'))
                    self.assertEqual(report['payload_bytes'], len(file_timeouts.PAYLOAD))
                    self.assertEqual(report['sha256'], file_timeouts.DIGEST)
                self.assertEqual(set(report), expected)
                self.assertFalse(any(event[0] in ('material', 'observe') for event in self.trace))
                for event in self.trace:
                    if event[0] == 'create':
                        self.assertEqual(event[3]['template'], 'base')
                self.assert_cleaned_once()

    async def test_cleanup_keeps_cancellation_visible_and_attempts_other_known_instance(self):
        first = self.create('async', {})
        second = self.create('async', {})
        self.kill_failures[second.sandbox_id] = asyncio.CancelledError()
        with self.assertRaises(BaseExceptionGroup) as caught:
            await inputs.destroy_probes([first, second])
        self.assertEqual(len(caught.exception.exceptions), 1)
        self.assertIsInstance(caught.exception.exceptions[0], asyncio.CancelledError)
        self.assert_cleaned_once()

    async def test_timeout_matrix_keeps_its_arguments_and_separate_observation_budget(self):
        for module, per_mode in [(command_timeouts, 26), (file_timeouts, 80), (rpc_timeouts, 48)]:
            with self.subTest(probe=module.__name__):
                self.reset()
                report = await self.execute(module)
                self.assertEqual([len(row['cases']) for row in report['results']], [per_mode, per_mode])
                for event in self.trace:
                    if event[0] == 'create':
                        self.assertEqual(event[3]['timeout'], 300)
                    if event[0] == 'observe':
                        self.assertEqual(event[3], dict(user='root', timeout=20))
                for sandbox in self.created:
                    cases = [event for event in self.trace if event[0] == 'case' and event[1] == sandbox.sandbox_id]
                    self.assertEqual(len(cases), per_mode)
                    observed = set()
                    for _, _, label, args in cases:
                        if label == 'run_case':
                            observed.add(args[-3:-1])
                        elif label == 'wait_deadline_case':
                            self.assertIn(args, [(True,), (False,)])
                            continue
                        else:
                            observed.add(args[-2:])
                    self.assertEqual(observed, {('omitted', None), ('zero', 0), ('negative', -1),
                        ('large' if module is file_timeouts else 'above_rpc_limit', 10_000_000)})

    async def test_timeout_case_failure_stops_matrix_and_retains_cleanup_and_primary_errors(self):
        for module, case in [(command_timeouts, 'run_case'), (file_timeouts, 'read_case'),
                             (rpc_timeouts, 'command_case')]:
            for mode in ('sync', 'async'):
                with self.subTest(probe=module.__name__, mode=mode):
                    self.reset()
                    ident = mode + '-1'
                    self.fail_case = (ident, case)
                    self.kill_failures[ident] = False
                    with self.assertRaises(BaseExceptionGroup) as caught:
                        await self.execute(module)
                    self.assertIsInstance(caught.exception.__context__, ValueError)
                    self.assertEqual(str(caught.exception.__context__), 'controlled case failure')
                    self.assertEqual([type(error) for error in caught.exception.exceptions], [RuntimeError])
                    self.assertEqual(len([event for event in self.trace
                        if event[:3] == ('case', ident, case)]), 1)
                    self.assertEqual([event[2] for event in self.trace
                        if event[:2] == ('observe', ident)], [1])
                    self.assert_cleaned_once()

    def test_shared_candidate_options_keep_all_five_fields(self):
        parser = argparse.ArgumentParser()
        data.guest_arguments(parser)
        values = parser.parse_args(['--template', 'candidate', '--guest-release', '/release',
            '--guest-manifest-sha256', 'a'*64, '--guest-configuration', '/config',
            '--guest-configuration-sha256', 'b'*64])
        self.assertEqual(vars(values), dict(template='candidate', guest_release='/release',
            guest_manifest_sha256='a'*64, guest_configuration='/config', guest_configuration_sha256='b'*64))


if __name__ == '__main__':
    unittest.main()
