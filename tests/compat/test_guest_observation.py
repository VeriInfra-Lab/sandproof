"""Pinned SDK classes with controlled operations; never create a real VM."""
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
from tools.system_acceptance import validate_probe
from tools.tests.test_system_acceptance import data_probe

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'system'))
import sdk_data_contract as contract


class GuestObservationOrchestrationTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.args = SimpleNamespace(api_url='http://127.0.0.1:1', sandbox_url='http://127.0.0.1:1',
            unprivileged_user='ubuntu', template='explicit-rust-candidate', guest_release='/private/release',
            guest_manifest_sha256='a'*64, guest_configuration='/private/config',
            guest_configuration_sha256='b'*64)
        self.expected = {role: dict(bytes=7, sha256='c'*64) for role in guest_probe.NAMES}
        self.identity = {'source_commit': 'd'*40}
        self.report = dict(schema_version=1, files=self.expected,
                           daemon=dict(pid=42, start_ticks=123, socket_inode=77))
        self.sync = SimpleNamespace(sandbox_id='sandbox-3', _envd_access_token='synthetic-sync',
                                    commands=SimpleNamespace(run=Mock(return_value=self.result())))
        self.async_ = SimpleNamespace(sandbox_id='sandbox-4', _envd_access_token='synthetic-async',
                                      commands=SimpleNamespace(run=AsyncMock(return_value=self.result())))
        self.create = self.replace(contract.Sandbox, 'create', Mock(return_value=self.sync))
        self.connect = self.replace(contract.Sandbox, 'connect', Mock(return_value=self.sync))
        self.kill = self.replace(contract.Sandbox, 'kill', Mock(return_value=True))
        self.async_create = self.replace(contract.AsyncSandbox, 'create', AsyncMock(return_value=self.async_))
        self.replace(contract.AsyncSandbox, 'connect', AsyncMock(return_value=self.async_))
        self.async_kill = self.replace(contract.AsyncSandbox, 'kill', AsyncMock(return_value=True))
        self.load = self.replace(guest_probe, 'load_expected', Mock(return_value=(self.identity, self.expected)))
        self.cases = self.replace(contract, 'sync_case', Mock(side_effect=data_probe()['sync']['cases']))
        self.async_cases = self.replace(contract, 'async_case', AsyncMock(side_effect=data_probe()['async']['cases']))
        self.resource = Mock()
        self.replace(contract._runpy, 'run_path', Mock(return_value={'record_resource_identity': self.resource}))
        self.stack.enter_context(patch.dict(os.environ, {'SANDPROOF_TEST_API_KEY': 'synthetic-owner'}))
        client = Mock()
        client.__enter__ = Mock(return_value=SimpleNamespace(get=Mock(return_value=SimpleNamespace(status_code=401))))
        client.__exit__ = Mock(return_value=False)
        self.replace(contract.httpx, 'Client', Mock(return_value=client))
        async_client = AsyncMock()
        async_client.__aenter__.return_value.get.return_value = SimpleNamespace(status_code=401)
        self.replace(contract.httpx, 'AsyncClient', Mock(return_value=async_client))

    def replace(self, target, name, value):
        return self.stack.enter_context(patch.object(target, name, value))

    def result(self, report=None):
        return SimpleNamespace(exit_code=0, stdout=json.dumps(self.report if report is None else report), stderr='')

    def run_probe(self):
        output = io.StringIO()
        with redirect_stdout(output):
            contract.exercise(self.args)
        return json.loads(output.getvalue())

    def test_both_sdk_modes_select_candidate_and_observe_before_and_after_without_replay(self):
        result = self.run_probe()
        self.assertEqual(result['template'], self.args.template)
        self.load.assert_called_once_with(self.args.guest_release, self.args.guest_manifest_sha256,
                                         self.args.guest_configuration, self.args.guest_configuration_sha256)
        for create in (self.create, self.async_create):
            create.assert_called_once()
            self.assertEqual(create.call_args.kwargs['template'], self.args.template)
            self.assertEqual(create.call_args.kwargs['retries'], 0)
            self.assertFalse(create.call_args.kwargs['allow_internet_access'])
        for kind, commands in [('sync', self.sync.commands), ('async', self.async_.commands)]:
            self.assertEqual(commands.run.call_count, 2)
            for invocation in commands.run.call_args_list:
                self.assertEqual(invocation.kwargs, {'user': 'root', 'timeout': 20})
                self.assertEqual(invocation.args, (guest_probe.command(self.expected),))
            self.assertEqual(result[kind]['rust_guest'], {'identity': self.identity, 'observation': self.report})
        self.kill.assert_called_once()
        self.assertEqual(self.kill.call_args.args, ('sandbox-3',))
        self.async_kill.assert_awaited_once()
        self.assertEqual(self.async_kill.call_args.args, ('sandbox-4',))

    def test_old_default_path_preserves_exact_c02_report_without_rust_observation(self):
        for name in ('template', 'guest_release', 'guest_manifest_sha256', 'guest_configuration',
                     'guest_configuration_sha256'):
            setattr(self.args, name, None)
        result = self.run_probe()
        self.assertEqual(result, data_probe())
        self.assertEqual(validate_probe('sdk_data_contract.py', result, 'unused'), {'status': 'pass'})
        self.load.assert_not_called()
        self.sync.commands.run.assert_not_called()
        self.async_.commands.run.assert_not_called()
        self.assertEqual(self.create.call_args.kwargs['template'], 'base')
        self.assertEqual(self.async_create.call_args.kwargs['template'], 'base')

    def test_partial_pins_or_missing_explicit_template_fail_before_any_vm(self):
        for field in ('template', 'guest_release', 'guest_manifest_sha256', 'guest_configuration',
                      'guest_configuration_sha256'):
            value = getattr(self.args, field)
            setattr(self.args, field, None)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.run_probe()
            setattr(self.args, field, value)
        self.load.assert_not_called()
        self.create.assert_not_called()
        self.async_create.assert_not_called()

    def test_first_bad_observation_prevents_data_cases_but_keeps_exact_cleanup(self):
        self.sync.commands.run.return_value = self.result({})
        with self.assertRaises(ValueError):
            self.run_probe()
        self.cases.assert_not_called()
        self.async_create.assert_not_called()
        self.create.assert_called_once()
        self.kill.assert_called_once()
        self.assertEqual(self.kill.call_args.args, ('sandbox-3',))

    def test_changed_async_process_cannot_pass_and_still_destroys_original_vm(self):
        changed = copy.deepcopy(self.report)
        changed['daemon']['start_ticks'] += 1
        self.async_.commands.run.side_effect = [self.result(), self.result(changed)]
        with self.assertRaisesRegex(AssertionError, 'changed during SDK cases'):
            self.run_probe()
        self.async_create.assert_awaited_once()
        self.assertEqual(self.async_cases.await_count, 2)
        self.assertEqual([item.args[2] for item in self.async_cases.await_args_list], ['root', 'ubuntu'])
        self.async_kill.assert_awaited_once()
        self.assertEqual(self.async_kill.call_args.args, ('sandbox-4',))


if __name__ == '__main__':
    unittest.main()
