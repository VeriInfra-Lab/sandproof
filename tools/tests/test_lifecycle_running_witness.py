"""Real public witness consumers with controlled SDK/file providers, no VM."""
import copy
import hashlib
from contextlib import ExitStack
from pathlib import Path
import sys
import threading
from types import SimpleNamespace as NS
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
                self.assertEqual(result, dict(session=client.session, same_ram_and_activation=True,
                    same_guest_process=True, same_authorization=True, counter=4,
                    guest_pid=41, guest_start_time_ticks=7, metadata_generation=2,
                    ram_nonce_sha256=hashlib.sha256(('a'*64).encode()).hexdigest(),
                    activation_nonce_sha256=hashlib.sha256(('b'*64).encode()).hexdigest()))
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


class _StartSubscription:
    def __init__(self,events,*,pid=41,identity_error=None,close_error=None):
        self.events=events;self.value=pid;self.identity_error=identity_error
        self.close_error=close_error;self.close_calls=0;self.kill=Mock()
        self.owner_thread=threading.get_ident();self.close_threads=[]

    @property
    def pid(self):
        self.events.append('pid')
        if self.identity_error is not None:raise self.identity_error
        return self.value

    def disconnect(self):
        self.close_calls+=1;self.events.append('disconnect')
        self.close_threads.append(threading.get_ident())
        if self.close_error is not None:raise self.close_error


class StartSubscriptionTests(unittest.TestCase):
    def fixture(self,*,pid=41,identity_error=None,close_error=None,start_error=None,after=None):
        old=lifecycle_fixture.RepeatedLifecycleRequestTests();client=old.client(owner='2')
        running=old.view(client,'running');events=[]
        handle=_StartSubscription(events,pid=pid,identity_error=identity_error,close_error=close_error)
        witness=dict(nonce='a'*64,activation_nonce='b'*64,metadata_generation=2,
                     counter=1,pid=41,start_time_ticks=7)
        sandbox=Mock();ready_error=RuntimeError('original ready failed')

        def command(*_,**kwargs):
            if kwargs.get('background') is True:
                events.append('start')
                if start_error is not None:raise start_error
                return handle
            events.append('ready')
            if after=='ready-rpc':raise ready_error
            return NS(exit_code=1 if after=='ready-exit' else 0)

        def observed(*_):
            events.append('witness')
            if after=='witness-rpc':raise ready_error
            return dict(witness,pid=42) if after=='witness-pid' else dict(witness,counter=2) if after=='witness-counter' else dict(witness)

        def read(*_):
            events.append('file')
            return 'different' if after=='file' else 'disk-counter:1'

        def query():
            events.append('query')
            return dict(running,current_version=1.0) if after=='query' else dict(running)

        sandbox.commands.run.side_effect=command;sandbox.files.read.side_effect=read
        client.sdk=Mock(return_value=sandbox);client.witness_command=Mock(side_effect=observed)
        client.query=Mock(side_effect=query)
        return NS(client=client,running=running,events=events,handle=handle,sandbox=sandbox,
                  witness=witness,ready_error=ready_error)

    def assert_no_guest_signal_or_replacement(self,cell,*,commands):
        self.assertEqual(cell.sandbox.commands.run.call_count,commands)
        starts=[c for c in cell.sandbox.commands.run.call_args_list if c.kwargs.get('background') is True]
        self.assertEqual(len(starts),1)
        self.assertEqual(starts[0].kwargs,dict(background=True,timeout=0))
        self.assertEqual(cell.handle.close_threads,[cell.handle.owner_thread]*cell.handle.close_calls)
        cell.handle.kill.assert_not_called();cell.sandbox.commands.kill.assert_not_called()

    def test_original_start_subscription_closes_before_ready_and_full_witness_continues(self):
        cell=self.fixture();result=cell.client.run_public_workload(cell.running)
        self.assertEqual(cell.events,['start','pid','disconnect','ready','witness','file','query'])
        self.assertEqual(cell.handle.close_calls,1)
        self.assert_no_guest_signal_or_replacement(cell,commands=2)
        ready=cell.sandbox.commands.run.call_args_list[1]
        self.assertTrue(ready.args[0].endswith(' 41'))
        self.assertGreater(ready.kwargs['timeout'],0);self.assertLessEqual(ready.kwargs['timeout'],30)
        self.assertEqual(cell.client.witness,cell.witness)
        self.assertEqual(result,dict(sdk='2.51.0',entry='owner-detail/original-sdk-constructor/public-gateway',
            path='/tmp/sandproof-unload-marker',ram_nonce_sha256=hashlib.sha256(('a'*64).encode()).hexdigest(),
            guest_pid=41,guest_start_time_ticks=7,counter=1))

    def test_invalid_returned_pid_still_closes_once_without_ready_or_new_start(self):
        for pid in (0,-1,True,41.0,'41',None,2**64):
            with self.subTest(pid_type=type(pid).__name__,pid=pid):
                cell=self.fixture(pid=pid)
                with self.assertRaises(AssertionError):cell.client.run_public_workload(cell.running)
                self.assertEqual(cell.handle.close_calls,1)
                self.assert_no_guest_signal_or_replacement(cell,commands=1)
                cell.client.witness_command.assert_not_called();cell.sandbox.files.read.assert_not_called()
                cell.client.query.assert_not_called()

    def test_pid_property_failure_preserves_original_error_and_closes_returned_subscription(self):
        error=RuntimeError('original Start PID unavailable');cell=self.fixture(identity_error=error)
        with self.assertRaises(RuntimeError) as caught:cell.client.run_public_workload(cell.running)
        self.assertIs(caught.exception,error);self.assertEqual(cell.handle.close_calls,1)
        self.assertEqual(cell.client.failure_step,'close-original-ram-start-subscription')
        self.assert_no_guest_signal_or_replacement(cell,commands=1)
        cell.client.witness_command.assert_not_called();cell.client.query.assert_not_called()

    def test_unknown_subscription_close_stops_before_ready_without_retry(self):
        error=RuntimeError('original subscription close unknown');cell=self.fixture(close_error=error)
        with self.assertRaises(RuntimeError) as caught:cell.client.run_public_workload(cell.running)
        self.assertIs(caught.exception,error);self.assertEqual(cell.handle.close_calls,1)
        self.assertEqual(cell.client.failure_step,'close-original-ram-start-subscription')
        self.assert_no_guest_signal_or_replacement(cell,commands=1)
        cell.client.witness_command.assert_not_called();cell.client.query.assert_not_called()

    def test_identity_and_close_failures_are_both_preserved_in_original_order(self):
        for invalid_pid in (False,True):
            with self.subTest(invalid_pid=invalid_pid):
                primary=RuntimeError('original identity failed');close=RuntimeError('original close failed')
                cell=self.fixture(pid=0 if invalid_pid else 41,
                    identity_error=None if invalid_pid else primary,close_error=close)
                with self.assertRaises(BaseExceptionGroup) as caught:cell.client.run_public_workload(cell.running)
                errors=caught.exception.exceptions;self.assertEqual(len(errors),2)
                if invalid_pid:self.assertIsInstance(errors[0],AssertionError)
                else:self.assertIs(errors[0],primary)
                self.assertIs(errors[1],close);self.assertEqual(cell.handle.close_calls,1)
                self.assert_no_guest_signal_or_replacement(cell,commands=1)
                cell.client.witness_command.assert_not_called();cell.client.query.assert_not_called()

    def test_unknown_start_without_returned_handle_is_not_replayed_or_closed_as_known(self):
        error=RuntimeError('original Start outcome unknown');cell=self.fixture(start_error=error)
        with self.assertRaises(RuntimeError) as caught:cell.client.run_public_workload(cell.running)
        self.assertIs(caught.exception,error);self.assertEqual(cell.handle.close_calls,0)
        self.assertEqual(cell.client.failure_step,'start-original-ram-process')
        self.assert_no_guest_signal_or_replacement(cell,commands=1)
        cell.client.witness_command.assert_not_called();cell.client.query.assert_not_called()

    def test_later_failures_keep_single_close_and_original_witness_checks(self):
        for after in ('ready-rpc','ready-exit','witness-rpc','witness-pid','witness-counter','file','query'):
            with self.subTest(after=after):
                cell=self.fixture(after=after)
                with self.assertRaises((AssertionError,RuntimeError)):cell.client.run_public_workload(cell.running)
                self.assertEqual(cell.handle.close_calls,1)
                self.assert_no_guest_signal_or_replacement(cell,commands=2)
                if after in ('ready-rpc','ready-exit'):cell.client.witness_command.assert_not_called()


if __name__ == '__main__':
    unittest.main()
