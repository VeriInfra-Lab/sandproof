"""Controlled SDK/route providers and an actual Linux relay process, no VM."""
import copy
from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tests/system'))
import lifecycle_command_disconnect as command


def fixture():
    witness = dict(nonce='1'*64, activation_nonce='2'*64, counter=1,
                   metadata_generation=0, pid=71, start_time_ticks=72)
    running = dict(session='original-session', phase='running')
    physical = dict(session=running['session'], context={'position': 0}, source={'original': 'VMM'})
    handle = Mock(pid=71)
    sdk = NS(commands=Mock(), files=Mock())
    sdk.commands.connect.return_value = handle
    client = NS(witness=copy.deepcopy(witness), deadline=time.monotonic()+10,
        session=running['session'], data_token='TESTONLYOLDTOKEN', sdk=Mock(return_value=sdk),
        query=Mock(return_value=running), assert_previous_tokens_closed=Mock(), witness_command=Mock())
    def read(_sdk, operation):
        result = copy.deepcopy(client.witness)
        if operation == 'advance':
            result['counter'] += 1
        return result
    client.witness_command.side_effect = read
    sdk.files.read.side_effect = lambda _: 'disk-counter:' + str(client.witness['counter']+1)
    runner = NS(services=NS(native_pools=[{'original': 'native'}]), confirm_running=Mock(return_value={'live': True}))
    source = dict(pid=91, start_time_ticks=92, boot_id='original-boot')
    route = dict(process=source, context={'original': 'protected helper route'})
    return NS(client=client, runner=runner, sdk=sdk, handle=handle, route=route,
              running=running, physical=physical)


@contextmanager
def providers(f):
    with ExitStack() as stack:
        routes = stack.enter_context(patch.object(command, '_live_routes', side_effect=[{}, {91: ('marker', f.route)}]))
        opened = stack.enter_context(patch.object(command, 'open_original_candidate_pidfd', return_value=999999))
        exited = stack.enter_context(patch.object(command, 'pidfd_exited', side_effect=[False, True]))
        marker = stack.enter_context(patch.object(command, 'original_json', return_value=copy.deepcopy(f.route)))
        closed = stack.enter_context(patch.object(command.os, 'close'))
        yield NS(routes=routes, opened=opened, exited=exited, marker=marker, closed=closed)


class CommandDisconnectTests(unittest.TestCase):
    def test_original_start_subscription_is_closed_once_and_ram_process_advances(self):
        f = fixture()
        with providers(f) as p:
            probe = command.CommandDisconnectProbe(f.runner, f.client)
            row = probe.observe(f.running, f.physical, cycle=0)
            self.assertTrue(row['original_relay_kernel_exit'])
            self.assertEqual(row['counter'], 2)
            self.assertEqual(row['ram_nonce_sha256'], hashlib.sha256(b'1'*64).hexdigest())
            self.assertNotIn('TESTONLYOLDTOKEN', json.dumps(row))
            self.assertNotIn('_activation_nonce', row)
            self.assertEqual(f.client.witness['counter'], 2)
            f.sdk.commands.connect.assert_called_once()
            self.assertEqual(f.sdk.commands.connect.call_args.args, (71,))
            f.handle.disconnect.assert_called_once_with()
            f.handle.kill.assert_not_called(); f.sdk.commands.run.assert_not_called()
            p.opened.assert_called_once_with(f.route['process'])
            f.runner.confirm_running.assert_called_once_with(f.client.session, f.physical, f.client.deadline)
            self.assertFalse(probe.close()['cleanup_unknown'])
            self.assertEqual(probe.close()['subscription_close_attempts'], 1)
            f.handle.disconnect.assert_called_once_with(); p.closed.assert_called_once_with(999999)

    def test_four_execution_observations_reconnect_one_pid_with_fresh_authorizations(self):
        f = fixture(); probe = command.CommandDisconnectProbe(f.runner, f.client)
        for cycle in range(4):
            if cycle:
                f.client.witness['counter'] += 1  # Existing assert_resumed_workload.
                f.client.witness['metadata_generation'] += 1
                f.client.witness['activation_nonce'] = str(cycle+2)*64
                f.client.data_token = 'TESTONLYNEW'+str(cycle)
                f.handle = Mock(pid=71); f.sdk.commands.connect.return_value = f.handle
            with providers(f):
                row = probe.observe(f.running, f.physical, cycle=cycle)
                self.assertEqual(row['counter'], 2+cycle*2)
                self.assertEqual(row['metadata_generation'], cycle)
        with patch.object(command.os, 'close'):
            self.assertFalse(probe.close()['cleanup_unknown'])
        self.assertEqual([call.args for call in f.sdk.commands.connect.call_args_list], [(71,)]*4)
        f.sdk.commands.run.assert_not_called()

    def test_resume_rejects_changed_ram_pid_nonce_generation_counter_or_old_token(self):
        for fault in ('pid', 'start_time_ticks', 'nonce', 'metadata_generation', 'activation_nonce', 'counter', 'token'):
            with self.subTest(fault=fault):
                f = fixture(); probe = command.CommandDisconnectProbe(f.runner, f.client)
                with providers(f): probe.observe(f.running, f.physical, cycle=0)
                f.sdk.commands.connect.reset_mock()
                f.client.witness.update(counter=3, metadata_generation=1, activation_nonce='3'*64)
                f.client.data_token='TESTONLYNEW'
                if fault in ('pid', 'start_time_ticks', 'counter'): f.client.witness[fault] += 1
                elif fault == 'nonce': f.client.witness[fault]='4'*64
                elif fault == 'metadata_generation': f.client.witness[fault]=2
                elif fault == 'activation_nonce': f.client.witness[fault]='2'*64
                else: f.client.data_token='TESTONLYOLDTOKEN'
                with self.assertRaises(ValueError): probe.observe(f.running, f.physical, cycle=1)
                f.sdk.commands.connect.assert_not_called()
                with patch.object(command.os, 'close'): probe.close()

    def test_missing_or_invalid_initial_witness_never_opens_a_connection(self):
        for fault in ('missing', 'bool', 'zero', 'nonce', 'extra'):
            f=fixture()
            if fault=='missing':del f.client.witness['activation_nonce']
            elif fault=='bool':f.client.witness['pid']=True
            elif fault=='zero':f.client.witness['start_time_ticks']=0
            elif fault=='nonce':f.client.witness['nonce']='z'*64
            else:f.client.witness['disk_nonce']='invented'
            with self.subTest(fault=fault), self.assertRaises(ValueError): command.CommandDisconnectProbe(f.runner, f.client)
            f.sdk.commands.connect.assert_not_called()

    def test_unknown_connect_is_not_replayed_or_converted_to_a_new_start(self):
        f=fixture(); f.sdk.commands.connect.side_effect=TimeoutError('unknown original Connect')
        probe=command.CommandDisconnectProbe(f.runner, f.client)
        with providers(f), self.assertRaises(TimeoutError): probe.observe(f.running, f.physical, cycle=0)
        self.assertEqual(probe.close()['subscription_close_attempts'], 0)
        f.sdk.commands.connect.assert_called_once(); f.sdk.commands.run.assert_not_called()

    def test_wrong_pid_ambiguous_or_changed_route_and_exited_relay_are_failures(self):
        for fault in ('pid', 'bool', 'ambiguous', 'marker', 'exited'):
            f=fixture(); probe=command.CommandDisconnectProbe(f.runner, f.client)
            with providers(f) as p, self.subTest(fault=fault):
                if fault=='pid':f.handle.pid=72
                elif fault=='bool':f.handle.pid=True
                elif fault=='ambiguous':p.routes.side_effect=[{}, {1: ('a', f.route), 2: ('b', f.route)}]
                elif fault=='marker':p.marker.return_value={'other': 'route'}
                else:p.exited.side_effect=None;p.exited.return_value=True
                with self.assertRaises(ValueError):probe.observe(f.running, f.physical, cycle=0)
                self.assertEqual(probe.rows, [])
                self.assertFalse(probe.close()['cleanup_unknown'])
                f.handle.disconnect.assert_called_once_with()
                f.client.witness_command.assert_not_called(); f.handle.kill.assert_not_called()

    def test_disconnect_error_is_retained_and_never_retried_or_signalled(self):
        f=fixture();f.handle.disconnect.side_effect=OSError('unknown close')
        probe=command.CommandDisconnectProbe(f.runner, f.client)
        with providers(f),self.assertRaises(OSError):probe.observe(f.running, f.physical, cycle=0)
        with patch.object(command.os, 'close'):
            result=probe.close();probe.close()
        self.assertTrue(result['cleanup_unknown']);self.assertEqual(result['failure_types'], ['OSError'])
        f.handle.disconnect.assert_called_once();f.handle.kill.assert_not_called()
        f.client.witness_command.assert_not_called();f.runner.confirm_running.assert_not_called()

    def test_changed_post_disconnect_witness_disk_or_public_view_cannot_succeed(self):
        for fault in ('read', 'advance', 'disk', 'view'):
            f=fixture();probe=command.CommandDisconnectProbe(f.runner, f.client)
            if fault=='read':f.client.witness_command.side_effect=lambda *_:dict(f.client.witness, nonce='3'*64)
            elif fault=='advance':f.client.witness_command.side_effect=lambda *_:copy.deepcopy(f.client.witness)
            elif fault=='disk':f.sdk.files.read.return_value='disk-counter:999';f.sdk.files.read.side_effect=None
            else:f.client.query.return_value=dict(f.running, phase='lost')
            with providers(f),self.subTest(fault=fault):
                with self.assertRaises(ValueError):probe.observe(f.running, f.physical, cycle=0)
                self.assertEqual(probe.rows, []);probe.close()
                f.handle.disconnect.assert_called_once();f.handle.kill.assert_not_called()

    def test_route_deadline_does_not_authorize_a_replacement_or_guest_signal(self):
        f=fixture();probe=command.CommandDisconnectProbe(f.runner, f.client)
        with providers(f) as p:
            p.routes.side_effect=[{}, {}]
            with patch.object(command.time, 'sleep', side_effect=TimeoutError('original route not observed')):
                with self.assertRaises(TimeoutError):probe.observe(f.running, f.physical, cycle=0)
            probe.close();p.opened.assert_not_called()
        f.sdk.commands.connect.assert_called_once();f.sdk.commands.run.assert_not_called();f.handle.kill.assert_not_called()


class LinuxRelayTests(unittest.TestCase):
    def test_real_relay_kernel_exit_does_not_end_a_separate_ram_process(self):
        # The route/SDK providers are controlled. Actual child identities,
        # stream EOF, pidfd exit and RAM-only counter/nonce are Linux facts.
        program = ('import json,os,secrets,sys; nonce=secrets.token_hex(32); counter=0\n'
                   'for line in sys.stdin:\n'
                   ' counter+=1;print(json.dumps(dict(pid=os.getpid(),nonce=nonce,counter=counter)),flush=True)\n')
        guest=subprocess.Popen([sys.executable, '-c', program],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
        relay=subprocess.Popen([sys.executable, '-c', 'import sys;sys.stdin.buffer.read()'],stdin=subprocess.PIPE)
        try:
            def read_guest():
                guest.stdin.write('advance\n');guest.stdin.flush()
                self.assertTrue(select.select([guest.stdout], [], [], 5)[0])
                return json.loads(guest.stdout.readline())
            before=read_guest();f=fixture()
            f.client.witness['pid']=guest.pid;f.handle.pid=guest.pid
            f.handle.disconnect.side_effect=relay.stdin.close
            boot=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
            ticks=int((Path('/proc')/str(relay.pid)/'stat').read_text().rsplit(')',1)[1].split()[19])
            route=dict(process=dict(pid=relay.pid,start_time_ticks=ticks,boot_id=boot))
            probe=command.CommandDisconnectProbe(f.runner,f.client)
            with patch.object(command,'_live_routes',side_effect=[{}, {relay.pid:('controlled-marker',route)}]), \
                    patch.object(command,'original_json',return_value=route):
                row=probe._subscription(f.physical,f.sdk)
            self.assertTrue(row['original_relay_kernel_exit']);self.assertEqual(relay.wait(timeout=5),0)
            after=read_guest()
            self.assertEqual((after['pid'],after['nonce']), (before['pid'],before['nonce']))
            self.assertEqual(after['counter'], before['counter']+1)
            self.assertIsNone(guest.poll());self.assertFalse(probe.close()['cleanup_unknown'])
            f.handle.disconnect.assert_called_once();f.handle.kill.assert_not_called()
        finally:
            # Exact test-owned children only; never a product/unknown PID.
            for child in (relay,guest):
                if child.stdin is not None and not child.stdin.closed:child.stdin.close()
                try:child.wait(timeout=5)
                except subprocess.TimeoutExpired:child.kill();child.wait(timeout=5)
                if child.stdout is not None:child.stdout.close()


class OriginalScenarioTests(unittest.TestCase):
    def test_three_original_cycles_and_cleanup_remain_and_failure_preserves_sessions(self):
        import tempfile
        import lifecycle_unload_scenario as scene
        import lifecycle_unload_runtime as runtime
        import lifecycle_unload_cow as cow
        import lifecycle_unload_retirement as retirement
        for fault in (False, True):
            events=[];clients={}
            lifetime=dict(started_at_ms=1,end_at_ms=2,max_end_at_ms=3,startup_deadline_ms=2)
            for owner in ('1','2'):
                c=Mock(spec=scene.PublicUnloadScenario);c.owner=owner;c.session='session-'+owner
                c.create_acceptance={};c.current_acceptance={};c.unload_acceptance={'key':{}}
                c.concurrent_unload_replies=[{},{}];c.resume_request={};c.witness={}
                c.timings=[dict(sent_monotonic=time.monotonic())];c.last_http_status=None;c.failure_step='controlled'
                c.timing_report.return_value=[];c.wait_running.return_value={'phase':'running'}
                c.wait_unloaded.return_value={'phase':'unloaded'};c.query.return_value={'phase':'running'}
                for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_resumed_workload'):
                    getattr(c,name).side_effect=lambda *_,n=name,o=owner,**__:events.append(n+'-'+o) or {}
                clients[owner]=c
            probe=Mock(spec=command.CommandDisconnectProbe)
            def observe(*_,cycle):
                events.append('disconnect-'+str(cycle))
                if fault and cycle==1:raise ValueError('unknown original subscription')
                return dict(cycle=cycle)
            probe.observe.side_effect=observe;probe.close.return_value={'cleanup_unknown':False}
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                calls=[]
                def record(session,*_,**__):
                    position=0 if session=='session-2' or not calls else 1
                    calls.append(session)
                    return dict(session=session,lifetime=lifetime,context={'position':position})
                runner=NS(root=Path(directory),owner_key='original',foreign_key='peer',
                    services=NS(sdk_url='http://127.0.0.1:1'),measurement_identity=lambda:{},
                    start=lambda _:None,create_parameters=lambda **_:{},record_running=Mock(side_effect=record),
                    unload_evidence=Mock(return_value=dict(facts=lifetime,original_position={'position':0},original_capture_responsibilities=1)),
                    restart_managers=Mock(return_value={'original_unload_operation':{}}),confirm_running=Mock(return_value={}),
                    destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),
                    finish=Mock(),fail_resume_gate=Mock(return_value='original responsibility preserved'),
                    close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
                source=Mock();source.verify_after_guest_write.return_value={}
                sources=stack.enter_context(patch.object(cow,'SealedSourceProbe',return_value=source))
                settled=stack.enter_context(patch.object(retirement,'wait_settled',return_value={'snapshot':{},'receipt':{}}))
                stack.enter_context(patch.object(retirement,'assert_replay_unchanged'))
                stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
                stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
                stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
                stack.enter_context(patch.object(command,'CommandDisconnectProbe',return_value=probe))
                args=NS(scenario='command-disconnect',scenario_timeout=60,activation_gate_timeout_ms=60000)
                if fault:
                    with self.assertRaisesRegex(ValueError,'unknown original subscription'):scene.exercise(args)
                    result=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                    self.assertFalse(result['complete']);self.assertEqual(len(result['known_sessions']),2)
                    clients['1'].destroy.assert_not_called();clients['2'].destroy.assert_not_called()
                    runner.settlement_evidence.assert_not_called();settled.assert_not_called()
                else:
                    result=scene.exercise(args)
                    self.assertTrue(result['complete']);self.assertEqual(len(result['cycles']),3)
                    self.assertEqual([r['cycle'] for r in result['command_disconnect']],[0,1,2,3])
                    self.assertEqual(clients['1'].run_public_workload.call_count,1)
                    self.assertEqual(clients['1'].resume.call_count,3)
                    self.assertEqual(clients['1'].unload.call_count,3)
                    self.assertEqual(clients['1'].assert_resumed_workload.call_count,3)
                    self.assertEqual(sources.call_count,3);self.assertEqual(settled.call_count,2)
                    self.assertLess(events.index('disconnect-0'),events.index('unload-1'))
                    self.assertLess(events.index('assert_resumed_workload-1'),events.index('disconnect-1'))
                    clients['1'].destroy.assert_called_once();clients['2'].destroy.assert_called_once()
                    runner.settlement_evidence.assert_called_once()
                probe.close.assert_called();runner.finish.assert_called_once()


if __name__ == '__main__':
    unittest.main()
