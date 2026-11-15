"""Real public witness consumers with controlled SDK/file providers, no VM."""
import copy
from contextlib import ExitStack
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_unload_scenario as scene
from tools.tests import test_lifecycle_unload_runtime as lifecycle_fixture


class RunningWitnessTests(unittest.TestCase):
    def fixture(self):
        old = lifecycle_fixture.RepeatedLifecycleRequestTests()
        client = old.client(owner='2')
        running = old.view(client, 'running')
        client.data_token = 'same-execution'
        client.witness = dict(nonce='a'*64, activation_nonce='b'*64, metadata_generation=2,
                              counter=3, pid=41, start_time_ticks=7)
        return client, running

    def exercise(self, fault='none'):
        client, running = self.fixture()
        self.assertIsNone(client.unload_acceptance)
        previous = copy.deepcopy(client.witness)
        observed = copy.deepcopy(previous)
        advanced = dict(previous, counter=4)
        if fault in ('nonce', 'activation_nonce', 'pid', 'start_time_ticks', 'metadata_generation', 'counter'):
            observed[fault] = 'c'*64 if isinstance(observed[fault], str) else observed[fault]+1
        if fault == 'advanced-identity': advanced['nonce'] = 'd'*64
        if fault == 'advanced-counter': advanced['counter'] = 5
        sandbox = Mock()
        sandbox.files.read.side_effect = ['changed' if fault == 'old-file' else 'disk-counter:3',
                                         'changed' if fault == 'new-file' else 'disk-counter:4']
        def sdk():
            if fault == 'authorization': client.data_token = 'other-execution'
            return sandbox
        current = dict(running, phase='cleaned') if fault == 'query' else running
        with ExitStack() as stack:
            stack.enter_context(patch.object(client, 'sdk', side_effect=sdk))
            witness = stack.enter_context(patch.object(client, 'witness_command', side_effect=[observed, advanced]))
            stack.enter_context(patch.object(client, 'query', return_value=current))
            tokens = stack.enter_context(patch.object(client, 'assert_previous_tokens_closed'))
            if fault == 'none':
                result = client.assert_running_workload(running)
                self.assertEqual(client.witness, advanced)
                self.assertEqual(witness.call_args_list[0].args[1], 'read')
                self.assertEqual(witness.call_args_list[1].args[1], 'advance')
                tokens.assert_called_once()
                self.assertEqual(result, dict(same_ram_and_activation=True, same_guest_process=True,
                                              same_authorization=True, counter=4))
                self.assertNotIn('same_lifetime', result)  # Native lifetime is a separate oracle.
            else:
                with self.assertRaises(AssertionError): client.assert_running_workload(running)

    def test_normal_running_without_unload_uses_current_acceptance_and_original_witness(self):
        self.exercise()

    def test_full_ram_activation_process_and_counter_identity_is_required(self):
        for fault in ('nonce', 'activation_nonce', 'pid', 'start_time_ticks', 'metadata_generation',
                      'counter', 'advanced-identity', 'advanced-counter'):
            with self.subTest(fault=fault): self.exercise(fault)

    def test_same_authorization_both_files_and_final_public_view_are_required(self):
        for fault in ('authorization', 'old-file', 'new-file', 'query'):
            with self.subTest(fault=fault): self.exercise(fault)

    def test_foreign_or_noncurrent_or_nonrunning_view_is_rejected_before_sdk(self):
        for field, value in (('session', 'other'), ('operation', {}), ('current_version', 2),
                             ('phase', 'cleaned'), ('unload_outcome', {'state':'original_continued', 'facts':{}})):
            client, running = self.fixture(); running[field] = value
            with self.subTest(field=field), patch.object(client, 'sdk') as sdk:
                with self.assertRaises(AssertionError): client.assert_running_workload(running)
                sdk.assert_not_called()

    def test_failed_unload_checker_still_requires_its_original_unload_and_lifetime(self):
        client, running = self.fixture()
        with patch.object(client, 'sdk') as sdk:
            with self.assertRaises((AssertionError, TypeError)):
                client.assert_continued_workload(running, {})
            sdk.assert_not_called()

    def test_missing_original_workload_witness_does_not_start_a_replacement(self):
        client, running = self.fixture(); client.witness = None
        with patch.object(client, 'sdk', return_value=Mock()), patch.object(client, 'witness_command', return_value={}):
            with self.assertRaises(AssertionError): client.assert_running_workload(running)


if __name__ == '__main__':
    unittest.main()
