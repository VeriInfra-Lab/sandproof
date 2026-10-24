"""Ordinary controlled tests for retained deployment exclusion; no root/VM/CI effects."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock,Mock,patch

SYSTEM=Path(__file__).resolve().parents[2]/'tests/system'
sys.path.insert(0,str(SYSTEM))
SPEC=importlib.util.spec_from_file_location('unload_runtime_checks',SYSTEM/'lifecycle_unload_runtime.py')
runtime=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)
SCENE_SPEC=importlib.util.spec_from_file_location('unload_scenario_checks',SYSTEM/'lifecycle_unload_scenario.py')
scene=importlib.util.module_from_spec(SCENE_SPEC)
SCENE_SPEC.loader.exec_module(scene)


class RetainedExclusionTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name)
        self.identity='1'*64
        self.args=SimpleNamespace(ci_state_root=self.root)
        self.quota=dict(host_lease_identity=self.identity,ci_state_root=str(self.root),mount=str(self.root),deployment_control=str(self.root/'run-evidence'))
        for name in ('paused','exclusive-host'):
            path=self.root/name
            path.write_text(json.dumps({'e2e':self.identity}));path.chmod(0o600)

    def test_only_existing_exact_exclusion_can_be_reused(self):
        with patch.object(runtime,'HostLease') as factory:
            self.assertIs(runtime.retained_deployment_lease(self.args,self.quota),factory.return_value)
            factory.assert_called_once_with(self.root,self.identity)
            factory.return_value.pause.assert_not_called()
            factory.return_value.release.assert_not_called()

    def test_foreign_exclusion_does_not_construct_or_replace_a_lease(self):
        (self.root/'exclusive-host').write_text(json.dumps({'e2e':'another-operator'}))
        with patch.object(runtime,'HostLease') as factory,self.assertRaises(ValueError):
            runtime.retained_deployment_lease(self.args,self.quota)
        factory.assert_not_called()
        self.assertEqual(json.loads((self.root/'exclusive-host').read_text()),{'e2e':'another-operator'})

    def test_missing_original_exclusion_is_not_created_from_receipt(self):
        (self.root/'exclusive-host').unlink()
        with patch.object(runtime,'HostLease') as factory,self.assertRaises((ValueError,FileNotFoundError)):
            runtime.retained_deployment_lease(self.args,self.quota)
        factory.assert_not_called()
        self.assertFalse((self.root/'exclusive-host').exists())

    def test_success_stops_managers_but_retains_exclusion_for_disk_teardown(self):
        state=object.__new__(runtime.UnloadRuntime)
        state.services=Mock()
        state.services.processes.audit.return_value={'host_slots':[]}
        state.services.native_pools=[dict(uid_base=3100000,bounded_host=dict(maintenance=dict(max_host_slots=2)))]
        state.running={'source':{'pid':123,'start_time_ticks':1,'boot_id':'test'}}
        state.physical_complete=True
        state.gc_complete=True
        state.restore_gate=None
        state.lease=Mock(identity=self.identity)
        state.quota={'deployment_control':str(self.root)}
        report={'complete':True}
        sdk=SimpleNamespace(processes_in_uid_range=lambda _first,_last:[])
        with patch.object(runtime,'original_process',return_value=False),patch.dict(sys.modules,{'sdk_c02_acceptance':sdk}):
            state.finish(report)
        state.services.stop_all.assert_called_once_with()
        state.lease.release.assert_not_called()
        self.assertEqual(report['host_lease'],'retained-until-exact-filesystem-teardown')
        stopped=json.loads((self.root/'scenario-stopped.json').read_text())
        self.assertEqual(stopped['host_lease_identity'],self.identity)
        self.assertEqual(stopped['physical_audit'],'Free')
        self.assertTrue(stopped['managers_exited'])


class ScenarioDiagnosticsTests(unittest.TestCase):
    def client(self):
        import time
        return scene.PublicUnloadScenario('http://127.0.0.1:12345', 'API_SECRET',
            'FOREIGN_SECRET', 'deployment', time.monotonic()+60)

    def assert_safe_stack(self, report):
        self.assertGreater(len(report['failure_stack']), 0)
        self.assertLessEqual(len(report['failure_stack']), 16)
        for frame in report['failure_stack']:
            self.assertEqual(set(frame), {'file', 'function', 'line'})
            self.assertEqual(Path(frame['file']).name, frame['file'])
            self.assertIs(type(frame['line']), int)
            self.assertGreater(frame['line'], 0)

    def test_http_failure_records_status_without_request_response_or_message(self):
        client=self.client()
        client.step('unload-acceptance', 202)
        with patch.object(scene, 'fetch', return_value=(503, b'BODY_SECRET')):
            try:
                client.read_json('GET', '/private?token=URL_SECRET', key='API_SECRET',
                    body={'hidden':'BODY_SECRET'}, status=202)
            except AssertionError as error:
                report=scene.failure_details(error, client, 'public-unload')
            else:
                self.fail('original strict HTTP assertion must reject 503')
        self.assertEqual(report['failure_step'], 'unload-acceptance')
        self.assertEqual(report['last_http_status'], {'expected':202, 'actual':503})
        self.assert_safe_stack(report)
        encoded=json.dumps(report)
        for secret in ('BODY_SECRET', 'API_SECRET', 'FOREIGN_SECRET', 'URL_SECRET',
                '/private', 'returned 503', str(SYSTEM)):
            self.assertNotIn(secret, encoded)

    def test_direct_file_status_is_recorded_before_original_strict_assertion(self):
        client=self.client()
        client.session='sandbox-a'
        client.data_token='TOKEN_SECRET'
        factory=MagicMock()
        factory.return_value.__enter__.return_value.get.return_value.status_code=403
        with patch.dict(sys.modules, {'httpx':SimpleNamespace(Client=factory)}):
            try:
                client.assert_data_closed()
            except AssertionError as error:
                report=scene.failure_details(error, client, 'public-unload')
            else:
                self.fail('original old-token assertion must reject 403')
        self.assertEqual(report['failure_step'], 'old-data-token')
        self.assertEqual(report['last_http_status'], {'expected':401, 'actual':403})
        self.assert_safe_stack(report)
        self.assertNotIn('TOKEN_SECRET', json.dumps(report))

    def test_exercise_keeps_false_report_and_reraises_without_exception_message(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            runner=SimpleNamespace(root=root, services=object(), owner_key='API_SECRET',
                foreign_key='FOREIGN_SECRET', finish=Mock(),measurement_identity=lambda:{},
                fail_resume_gate=Mock(return_value='original responsibility preserved'),close_restore_gate=Mock())
            error=AssertionError('BODY_SECRET API_SECRET TOKEN_SECRET')
            args=SimpleNamespace(scenario_timeout=60,scenario='normal',activation_gate_timeout_ms=60000)
            module=SimpleNamespace(prepare=lambda _args:runner)
            with patch.dict(sys.modules, {'lifecycle_unload_runtime':module}), \
                    patch.object(scene, 'check_center_configuration', side_effect=error), \
                    self.assertRaises(AssertionError) as caught:
                scene.exercise(args)
            self.assertIs(caught.exception, error)
            report=json.loads((root/'lifecycle-unload-report.json').read_text())
            self.assertFalse(report['complete'])
            self.assertEqual(report['phase'], 'configuration')
            self.assertEqual(report['failure_type'], 'AssertionError')
            self.assertEqual(report['failure_step'], 'configuration')
            self.assertIsNone(report['last_http_status'])
            self.assert_safe_stack(report)
            runner.finish.assert_called_once()
            self.assertFalse(runner.finish.call_args.args[0]['complete'])
            for secret in ('BODY_SECRET', 'API_SECRET', 'TOKEN_SECRET', 'FOREIGN_SECRET'):
                self.assertNotIn(secret, json.dumps(report))


class RepeatedLifecycleRequestTests(unittest.TestCase):
    def client(self,owner='1'):
        import time
        client=scene.PublicUnloadScenario('http://127.0.0.1:12345','key','other',
            'deployment',time.monotonic()+60,owner=owner)
        client.window=1;client.session='original-session';client.sequence=1
        client.create_acceptance=dict(session=client.session,key=client.key(1),accepted_version=1)
        client.current_acceptance=client.create_acceptance
        return client

    def view(self,client,phase):
        accepted=client.current_acceptance
        return dict(session=client.session,operation=accepted['key'],
            current_version=accepted['accepted_version'],phase=phase,
            unload_outcome=({'state':'unloaded','facts':{}} if phase=='unloaded' else {'state':'pending'} if phase=='unloading' else None),create_refusal=None)

    def test_two_restore_cycles_keep_session_owner_and_monotone_original_keys(self):
        client=self.client()
        calls=[]
        def accepted(method,path,*,key,body,status):
            self.assertEqual((method,key,status),('POST','key',202))
            calls.append((path,dict(body)))
            return dict(session=client.session,key=client.key(body['sequence']),
                accepted_version=body['expected_version']+1)
        with patch.object(client,'read_json',side_effect=accepted):
            client.unload(self.view(client,'running'))
            client.resume(self.view(client,'unloaded'))
            client.unload(self.view(client,'running'))
            client.resume(self.view(client,'unloaded'))
            client.destroy(self.view(client,'running'))
        self.assertEqual([row[1]['sequence'] for row in calls],[2,3,4,5,6])
        self.assertEqual([row[1]['expected_version'] for row in calls],[1,2,3,4,5])
        self.assertEqual([row[0].rsplit('/',1)[-1] for row in calls],
            ['unload','resume','unload','resume','destroy'])
        self.assertTrue(all('/sessions/original-session/' in row[0] for row in calls))
        self.assertEqual(client.current_acceptance['key']['owner'],'1')

    def test_second_owner_does_not_reuse_first_owner_operation_identity(self):
        first,second=self.client(),self.client('2')
        self.assertNotEqual(first.key(1),second.key(1))
        self.assertEqual(second.key(1)['owner'],'2')

    def test_concurrent_unload_uses_one_original_key_and_two_identical_bodies(self):
        client=self.client()
        calls=[]
        def accepted(method,path,*,key,body,status):
            calls.append((method,path,key,dict(body),status))
            return dict(session=client.session,key=client.key(body['sequence']),
                accepted_version=body['expected_version']+1)
        with patch.object(client,'read_json',side_effect=accepted):
            client.unload(self.view(client,'running'),concurrent_duplicate=True)
        self.assertEqual(len(calls),2)
        self.assertEqual(calls[0],calls[1])
        self.assertEqual(calls[0][3],dict(window=1,sequence=2,expected_version=1))
        self.assertEqual(client.sequence,2)
        self.assertEqual(len(client.timings),1)
        self.assertEqual(client.concurrent_unload_replies,[client.unload_acceptance]*2)

    def test_concurrent_reply_disagreement_is_not_retried_or_newly_accepted(self):
        client=self.client()
        before=client.current_acceptance.copy()
        replies=[dict(session=client.session,key=client.key(2),accepted_version=value)
            for value in (2,3)]
        with patch.object(client,'read_json',side_effect=replies) as send:
            with self.assertRaises(AssertionError):
                client.unload(self.view(client,'running'),concurrent_duplicate=True)
        self.assertEqual(send.call_count,2)
        self.assertEqual(client.current_acceptance,before)
        self.assertEqual(client.sequence,2)
        self.assertIsNone(client.timings[0]['accepted_monotonic'])

    def test_uncertain_resume_does_not_retry_or_allocate_a_replacement_key(self):
        client=self.client()
        client.sequence=2
        client.current_acceptance=dict(session=client.session,key=client.key(2),accepted_version=2)
        before=client.current_acceptance.copy()
        with patch.object(client,'read_json',side_effect=TimeoutError) as call,self.assertRaises(TimeoutError):
            client.resume(self.view(client,'unloaded'))
        self.assertEqual(call.call_count,1)
        self.assertEqual(client.resume_request,dict(window=1,sequence=3,expected_version=2))
        self.assertEqual(client.current_acceptance,before)

    def test_success_without_gc_oracle_never_authorizes_manager_teardown(self):
        state=object.__new__(runtime.UnloadRuntime)
        state.physical_complete=True;state.gc_complete=False
        state.services=Mock()
        report=dict(complete=True)
        state.finish(report)
        self.assertEqual(report['cleanup'],'unknown-preserved-original-resources-and-host-lease')
        state.services.stop_all.assert_not_called()


class RestoreGateBindingTests(unittest.TestCase):
    def setUp(self):
        self.expected=dict(action={'identity':{'generation':2},
            'command':{'logical':{'command':{'ActivateVm':dict(sandbox=1,epoch=1,generation=2,vm=1)}},'attempt':0}},
            candidate=dict(resource_id='vm-original-1',pid=123,start_time_ticks=456,boot_id='boot'),
            namespace=dict(name='vm-original-1',device=7,inode=8))
        self.record=dict(schema_version=1,run_identity='1'*64,token='T'*64,**self.expected)

    def test_exact_private_run_and_candidate_is_admitted_without_executing_observer(self):
        self.assertIs(runtime.validate_restore_gate(self.record,self.expected,'1'*64),self.record)

    def test_foreign_run_action_vmm_namespace_or_unbounded_secret_is_rejected(self):
        import copy
        mutations=[('run_identity','2'*64),('action',{}),('candidate',{}),('namespace',{}),
            ('token','T'*129),('token','T'*32+'\n'),('schema_version',True)]
        for key,value in mutations:
            changed=copy.deepcopy(self.record);changed[key]=value
            with self.subTest(key=key),self.assertRaises(ValueError):
                runtime.validate_restore_gate(changed,self.expected,'1'*64)
        changed=copy.deepcopy(self.record);changed['command']='arbitrary-code'
        with self.assertRaises(ValueError):runtime.validate_restore_gate(changed,self.expected,'1'*64)
        changed=copy.deepcopy(self.record);changed['source']=changed.pop('candidate')
        with self.assertRaises(ValueError):runtime.validate_restore_gate(changed,self.expected,'1'*64)

    def test_guest_witness_programs_are_syntax_valid_without_running_them(self):
        for name in ('RAM_WITNESS_SERVER','RAM_WITNESS_READY','RAM_WITNESS_CLIENT'):
            compile(getattr(scene,name),'<original-guest-witness>','exec')

    def test_recovered_running_requires_exact_gate_candidate_and_namespace(self):
        receipt=dict(candidate=self.expected['candidate'],namespace=self.expected['namespace'])
        running=dict(source=dict(self.expected['candidate']),namespace=dict(self.expected['namespace']))
        runtime.assert_same_restore_candidate(running,receipt)
        # Snapshot restoration can reproduce guest PID/start ticks and RNG
        # output. Those identical values must not excuse a different host VMM.
        running['guest_witness']=dict(nonce='a'*64,activation_nonce='b'*64,pid=9,start_time_ticks=10)
        receipt['guest_witness']=dict(running['guest_witness'])
        for key,value in (('resource_id','vm-other-1'),('pid',124),
                          ('start_time_ticks',457),('boot_id','other-boot')):
            changed=dict(running,source=dict(running['source'],**{key:value}))
            with self.subTest(candidate_field=key),self.assertRaises(ValueError):
                runtime.assert_same_restore_candidate(changed,receipt)
        for key,value in (('name','vm-other-1'),('device',9),('inode',10)):
            changed=dict(running,namespace=dict(running['namespace'],**{key:value}))
            with self.subTest(namespace_field=key),self.assertRaises(ValueError):
                runtime.assert_same_restore_candidate(changed,receipt)
        with self.assertRaises(ValueError):runtime.assert_same_restore_candidate(None,receipt)
        changed=dict(running);changed.pop('namespace')
        with self.assertRaises(ValueError):runtime.assert_same_restore_candidate(changed,receipt)


class DuplicateUnloadHistoryTests(unittest.TestCase):
    def test_original_history_has_one_business_operation_and_exact_capture(self):
        key=dict(deployment='deployment',owner='1',window=1,sequence=2)
        acceptance=dict(session='session',key=key,accepted_version=2)
        binding=dict(session='session',key=key)
        unload=dict(binding=binding,accepted_version=2,
            operation=dict(kind='unload',source_incarnation=1,capture=1))
        operations=[dict(binding=dict(session='session',key=dict(key,sequence=1)),
            operation=dict(kind='create')),unload]
        audit=dict(sandboxes=[dict(create=dict(binding=dict(session='session')),operations=operations)])
        saved=dict(unload_outcome=dict(state='unloaded',facts=dict(capture=1,source_incarnation=1)))
        result=runtime.original_unload_operation(audit,acceptance,saved)
        self.assertEqual(result['matching_operations'],1)
        self.assertEqual(result['original_session_operations'],2)
        import copy
        duplicate=copy.deepcopy(audit)
        duplicate['sandboxes'][0]['operations'].append(copy.deepcopy(unload))
        with self.assertRaises(ValueError):runtime.original_unload_operation(duplicate,acceptance,saved)
        for key,value in (('capture',2),('source_incarnation',2)):
            changed=copy.deepcopy(audit)
            changed['sandboxes'][0]['operations'][1]['operation'][key]=value
            with self.subTest(field=key),self.assertRaises(ValueError):
                runtime.original_unload_operation(changed,acceptance,saved)


class LifecycleTimingTests(unittest.TestCase):
    def client(self):
        client=scene.PublicUnloadScenario('http://127.0.0.1:12345','key','other','deployment',100)
        client.session='original-session'
        client.current_acceptance=dict(key={'sequence':1},accepted_version=1)
        return client

    def test_acceptance_is_separate_from_public_and_physical_completion(self):
        client=self.client()
        with patch.object(scene.time,'monotonic',side_effect=[10,12,17,20]):
            client.operation_sent('resume')
            client.operation_accepted()
            accepted=client.timing_report()[0]
            self.assertEqual(accepted['send_to_accept_seconds'],2)
            self.assertFalse(accepted['complete'])
            self.assertIsNone(accepted['send_to_public_completion_seconds'])
            client.operation_completed('running')
            client.operation_evidence_completed()
        sample=client.timing_report()[0]
        self.assertEqual(sample['send_to_public_completion_seconds'],7)
        self.assertEqual(sample['accept_to_public_completion_seconds'],5)
        self.assertEqual(sample['send_to_evidence_completion_seconds'],10)
        self.assertEqual(sample['public_completion_to_evidence_seconds'],3)
        self.assertEqual(sample['acceptance'],client.current_acceptance)

    def test_wrong_phase_replay_and_unknown_acceptance_cannot_fabricate_completion(self):
        client=self.client()
        with patch.object(scene.time,'monotonic',side_effect=[10,12,17]):
            client.operation_sent('unload')
            client.operation_accepted()
            client.operation_completed('running')
            self.assertFalse(client.timing_report()[0]['complete'])
            client.operation_completed('unloaded')
            client.operation_completed('unloaded')
        self.assertEqual(client.timing_report()[0]['send_to_public_completion_seconds'],7)
        with patch.object(scene.time,'monotonic',return_value=20):
            client.operation_sent('destroy')
            client.operation_completed('cleaned')
        unknown=client.timing_report()[-1]
        self.assertIsNone(unknown['send_to_accept_seconds'])
        self.assertFalse(unknown['complete'])


class WitnessMetadataBarrierTests(unittest.TestCase):
    def setUp(self):
        self.baseline=dict(nonce='a'*64,activation_nonce='b'*64,metadata_generation=2,
            counter=3,pid=4,start_time_ticks=5)

    def test_unchanged_old_observations_wait_until_exact_new_generation(self):
        current=dict(self.baseline,metadata_generation=3,activation_nonce='c'*64)
        read=Mock(side_effect=[dict(self.baseline),dict(self.baseline),current])
        with patch.object(scene.time,'monotonic',return_value=1):
            self.assertIs(scene.wait_witness_generation(read,self.baseline,10),current)
        self.assertEqual(read.call_count,3)

    def test_identity_state_regression_jump_or_nonce_reuse_rejects_immediately(self):
        mutations=[('nonce','d'*64),('counter',4),('pid',6),('start_time_ticks',7),
            ('metadata_generation',1),('metadata_generation',4),
            ('activation_nonce','c'*64)]
        values=[dict(self.baseline,**{key:value}) for key,value in mutations]
        values.append(dict(self.baseline,metadata_generation=3))
        for value in values:
            read=Mock(return_value=value)
            with self.subTest(value=value),patch.object(scene.time,'monotonic',return_value=1):
                with self.assertRaises(ValueError):
                    scene.wait_witness_generation(read,self.baseline,10)
            read.assert_called_once_with()

    def test_old_generation_is_bounded_by_original_deadline(self):
        read=Mock(return_value=dict(self.baseline))
        with patch.object(scene.time,'monotonic',side_effect=[1,1,10]):
            with self.assertRaises(TimeoutError):
                scene.wait_witness_generation(read,self.baseline,10)
        read.assert_called_once_with()

    def test_late_success_cannot_extend_original_deadline(self):
        read=Mock(return_value=dict(self.baseline,metadata_generation=3,activation_nonce='c'*64))
        with patch.object(scene.time,'monotonic',side_effect=[1,10]):
            with self.assertRaises(TimeoutError):
                scene.wait_witness_generation(read,self.baseline,10)
        read.assert_called_once_with()


if __name__=='__main__':
    unittest.main()

class SourceRowsTests(unittest.TestCase):
    def setUp(self):
        import sqlite3
        import hashlib
        import time
        self.hashlib=hashlib
        self.conn=sqlite3.connect(':memory:')
        self.addCleanup(self.conn.close)
        self.deadline=time.monotonic()+10
        self.config=dict(max_work_bytes=65536,max_configuration_bytes=16384)
        self.host=dict(native=dict(maintenance=dict(max_operations_per_sandbox=8,
            capacity=dict(max_sql_value_bytes=16*1024**2,max_sql_rows=8192))),
            inbox=dict(max_sessions=1,node=dict(node_id='original')),source_observation=self.config)
        self.reserve=65536+4*16384+8192+131072+4*16384+4096+8192
        self.conn.executescript('CREATE TABLE controller_meta(id INTEGER,version INTEGER);'
            'INSERT INTO controller_meta VALUES(1,23);'
            'CREATE TABLE controller_source_work(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);'
            'CREATE TABLE controller_source_work_records(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER,work_key BLOB);')
        meta=json.dumps(dict(config=self.config,node=self.host['inbox']['node'],max_sessions=1)).encode()
        self.conn.execute('INSERT INTO controller_source_work VALUES(1,?,?,8192)',
            (meta,hashlib.sha256(meta).digest()))
        for index in range(1,7):
            self.conn.execute('INSERT INTO controller_source_work_records VALUES(?,?,?,?,NULL)',
                (index,b'null',hashlib.sha256(b'null').digest(),self.reserve))

    def test_vacant_reserved_rows_are_read_without_mutation(self):
        before=self.conn.total_changes
        self.assertEqual(runtime.checked_source_records(self.conn,self.host,self.deadline),[])
        self.assertEqual(self.conn.total_changes,before)

    def test_wrong_format_missing_row_and_changed_reserve_are_rejected(self):
        for statement in ('UPDATE controller_meta SET version=22',
                'DELETE FROM controller_source_work_records WHERE id=6',
                'UPDATE controller_source_work_records SET reserve_bytes=1 WHERE id=1'):
            with self.subTest(statement=statement):
                self.conn.execute('SAVEPOINT changed')
                self.conn.execute(statement)
                with self.assertRaises(ValueError):
                    runtime.checked_source_records(self.conn,self.host,self.deadline)
                self.conn.execute('ROLLBACK TO changed');self.conn.execute('RELEASE changed')

    def test_occupied_row_binds_checksum_and_original_work_digest(self):
        anchor=dict(original='work')
        key=self.hashlib.sha256(json.dumps(anchor,separators=(',',':')).encode()).digest()
        record=dict(work=dict(anchor=anchor,requirement=dict(work_binding=list(key))),
            observation=None,capture=None,restore=None,retirement=None)
        encoded=json.dumps(record,separators=(',',':')).encode()
        self.conn.execute('UPDATE controller_source_work_records SET state_json=?,sha256=?,work_key=? WHERE id=1',
            (encoded,self.hashlib.sha256(encoded).digest(),key))
        self.assertEqual(runtime.checked_source_records(self.conn,self.host,self.deadline),[record])
        self.conn.execute('UPDATE controller_source_work_records SET work_key=? WHERE id=1',(bytes(32),))
        with self.assertRaises(ValueError):
            runtime.checked_source_records(self.conn,self.host,self.deadline)

    def test_missing_mandatory_retirement_field_is_not_an_old_layout_fallback(self):
        anchor=dict(original='work')
        key=self.hashlib.sha256(json.dumps(anchor,separators=(',',':')).encode()).digest()
        record=dict(work=dict(anchor=anchor,requirement=dict(work_binding=list(key))),
            observation=None,capture=None,restore=None)
        encoded=json.dumps(record,separators=(',',':')).encode()
        self.conn.execute('UPDATE controller_source_work_records SET state_json=?,sha256=?,work_key=? WHERE id=1',
            (encoded,self.hashlib.sha256(encoded).digest(),key))
        with self.assertRaises(ValueError):
            runtime.checked_source_records(self.conn,self.host,self.deadline)


class ActivationGateProtocolTests(unittest.TestCase):
    def test_configuration_rejects_invalid_bounds_before_stopping_host(self):
        gate=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime)
        gate.restore_gate=None
        gate.lease=SimpleNamespace(identity='1'*64)
        gate.runs={'session':dict(context=dict(binding=dict(generation=1)),local=dict(sandbox=3))}
        manager=Mock()
        processes=SimpleNamespace(children={'host-0':manager},stop=Mock())
        gate.services=SimpleNamespace(native_pools=[dict(namespace='0123456789abcdef')],processes=processes)
        import time
        for timeout in (True,999,120001,1.0):
            with self.subTest(timeout=timeout),self.assertRaises(ValueError):
                gate.arm_restore_gate('session',time.monotonic()+10,timeout)
        processes.stop.assert_not_called()
        manager.poll.assert_not_called()

    def test_ready_and_command_bind_exact_digest_run_and_finite_choice(self):
        digest='2'*64
        runtime.validate_gate_ready(dict(schema_version=1,observation_sha256=digest),digest)
        self.assertEqual(runtime.gate_command('1'*64,digest,'continue')['command'],'continue')
        for value in (dict(schema_version=True,observation_sha256=digest),
                dict(schema_version=1,observation_sha256='3'*64),
                dict(schema_version=1,observation_sha256=digest,token='PRIVATE')):
            with self.assertRaises(ValueError):runtime.validate_gate_ready(value,digest)
        for command in ('advance','init','',None):
            with self.assertRaises(ValueError):runtime.gate_command('1'*64,digest,command)

    def test_atomic_command_publication_never_overwrites_or_accepts_stage(self):
        import os
        import stat
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            descriptor=os.open(root,os.O_RDONLY|os.O_DIRECTORY)
            self.addCleanup(os.close,descriptor)
            config=runtime.gate_configuration('1'*64,'sb-0123456789abcdef-3',2,1000)
            runtime.gate_publish(descriptor,'config.json',config)
            with self.assertRaises(FileExistsError):runtime.gate_publish(descriptor,'config.json',config)
            value=runtime.gate_command('1'*64,'2'*64,'continue')
            runtime.gate_publish(descriptor,'command.json',value)
            original=(root/'command.json').read_bytes()
            self.assertEqual(json.loads(original),value)
            self.assertFalse((root/'command-stage.json').exists())
            self.assertEqual(stat.S_IMODE((root/'command.json').stat().st_mode),0o600)
            with self.assertRaises(OSError):runtime.gate_publish(descriptor,'command.json',value)
            self.assertEqual((root/'command.json').read_bytes(),original)
            self.assertEqual((root/'command.json').stat().st_nlink,1)
            with self.assertRaises(ValueError):runtime.gate_publish(descriptor,'ready.json',value)

    def test_cleanup_preserves_gate_until_original_product_and_exit_settle(self):
        gate=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime)
        gate.restore_gate=dict(command='continue',command_sent=True,exit86_observed=False)
        gate.gc_complete=gate.physical_complete=False
        gate._gate_files_unchanged=Mock()
        with self.assertRaises(ValueError):gate.cleanup_restore_gate()
        gate._gate_files_unchanged.assert_not_called()

    def test_normal_scenario_has_no_gate_files_or_fault_command(self):
        gate=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime)
        gate.restore_gate=None
        self.assertIsNone(gate.cleanup_restore_gate())
        self.assertEqual(gate.fail_resume_gate(),'no new gate command; original responsibility preserved')
        gate.close_restore_gate()

    def test_invalid_e7_deadline_never_prepares_or_starts_services(self):
        module=SimpleNamespace(prepare=Mock())
        args=SimpleNamespace(scenario='restore-activation-loss',activation_gate_timeout_ms=999,
            scenario_timeout=60)
        with patch.dict(sys.modules,{'lifecycle_unload_runtime':module}),self.assertRaises(ValueError):
            scene.exercise(args)
        module.prepare.assert_not_called()

    def test_recovery_requires_exit86_and_restarts_original_host_without_fault_environment(self):
        import time
        for code in (0,86):
            gate=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime)
            original=Mock(pid=101)
            original.wait.return_value=code
            recovered=Mock(pid=102)
            processes=SimpleNamespace(children={'host-0':original},
                bindings={'host-0':dict(config='/protected/host.json',uid=0,gid=0)},
                start=Mock(return_value=recovered))
            gate.restore_gate=dict(host=original,observation_sha256='2'*64)
            gate.release_resume_gate=Mock()
            gate._gate_files_unchanged=Mock()
            gate.services=SimpleNamespace(processes=processes,wait_ready=Mock())
            gate.configs={'host-0':dict(original=True)}
            receipt=dict(candidate=dict(pid=201,start_time_ticks=3,resource_id='vm-original',boot_id='boot'))
            with patch.object(runtime,'role_facts',return_value=gate.configs['host-0']), \
                    patch.object(runtime,'original_process',return_value=True):
                if code==86:
                    result=gate.recover_resume_gate(receipt,time.monotonic()+10)
                    self.assertEqual(result['exit_code'],86)
                    self.assertFalse(result['public_authorization_proven'])
                    processes.start.assert_called_once_with('host-0')
                    self.assertTrue(gate.restore_gate['exit86_observed'])
                else:
                    with self.assertRaises(ValueError):gate.recover_resume_gate(receipt,time.monotonic()+10)
                    processes.start.assert_not_called()
            gate.release_resume_gate.assert_called_once_with('continue')

    def test_exact_cleanup_removes_only_sealed_final_files_and_rejects_unknown_entries(self):
        import os
        from contextlib import ExitStack
        # Only privilege metadata is controlled; bytes, descriptors, hashes,
        # no-follow opens, single links and unlink identities are real files.
        real_stat,real_fstat=os.stat,os.fstat
        def root_metadata(value):
            names=('st_dev','st_ino','st_size','st_mtime_ns','st_ctime_ns','st_mode','st_uid','st_nlink')
            result={name:getattr(value,name) for name in names}
            result['st_uid']=0
            return SimpleNamespace(**result)
        for unknown in (False,True):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as guards:
                control=Path(directory)
                path=control/'restore-activation-gate';path.mkdir(mode=0o700)
                for name in runtime.GATE_FINAL_NAMES:
                    (path/name).write_text(json.dumps(dict(fixture=name)))
                    (path/name).chmod(0o600)
                if unknown:(path/'foreign.json').write_text('{}')
                descriptor=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
                gate=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime)
                gate.physical_complete=gate.gc_complete=True
                meta=real_fstat(descriptor);parent=real_stat(control)
                gate.restore_gate=dict(path=path,fd=descriptor,directory_identity=(meta.st_dev,meta.st_ino),
                    control_identity=(parent.st_dev,parent.st_ino),files={},command='continue',
                    command_sent=True,exit86_observed=True)
                guards.enter_context(patch.object(runtime.os,'stat',side_effect=lambda *a,**kw:root_metadata(real_stat(*a,**kw))))
                guards.enter_context(patch.object(runtime.os,'fstat',side_effect=lambda *a,**kw:root_metadata(real_fstat(*a,**kw))))
                try:
                    for name in runtime.GATE_FINAL_NAMES:
                        _,digest,identity=runtime.gate_json(path/name,16384 if name=='observation.json' else 1024,
                            directory_fd=descriptor)
                        gate.restore_gate['files'][name]=(digest,identity)
                    if unknown:
                        with self.assertRaises(ValueError):gate.cleanup_restore_gate()
                        self.assertEqual(set(os.listdir(descriptor)),runtime.GATE_FINAL_NAMES|{'foreign.json'})
                    else:
                        result=gate.cleanup_restore_gate()
                        self.assertEqual(result['protocol_names'],sorted(runtime.GATE_NAMES))
                        self.assertEqual(result['transient_stages_absent'],sorted(runtime.GATE_STAGE_NAMES))
                        self.assertTrue(result['private_token_record_removed'])
                        self.assertFalse(path.exists())
                        self.assertTrue(control.is_dir())
                finally:
                    gate.close_restore_gate()

class MaterialSettlementBindingTests(unittest.TestCase):
    def record(self,state='retiring'):
        full=dict(pause={},compatibility={},state_bytes=4096,memory_bytes=8192,disk_bytes=16384,
            api_timeout_ms=1000,worker_timeout_ms=2000)
        anchor=dict(local=dict(sandbox=1,owner=2),native=dict(generation=3))
        requirement=dict(work_binding=[7]*32)
        capture=dict(full=full,action=None,worker=None,completed=None,
            reserved_material_bytes=2*(4096+8192+16384)+2*262144+65536)
        original=[1,3,full,requirement,None,None,None]
        digest=lambda value:list(runtime.hashlib.sha256(value).digest())
        work=digest(json.dumps(original,ensure_ascii=False,separators=(',',':')).encode())
        raw=list(b'controlled original physical-port plan')
        plan=dict(bytes=raw,work_binding=work,plan_binding=digest(bytes(raw)),
            quiescence=dict(sandbox=1,owner=2,epoch=4,operation=5,workspace=6,
                worker_drained='Confirmed',data_connections_drained='Confirmed',
                host_resources_reclaimed='Confirmed',external_pins_released='Confirmed'))
        retirement=dict(state=state,plan=plan)
        if state=='settled':
            retirement['settlement']=dict(work_binding=work,plan_binding=plan['plan_binding'],
                released_material_bytes=capture['reserved_material_bytes'])
        return dict(work=dict(anchor=anchor,requirement=requirement),observation=None,
            capture=capture,restore=dict(material_use=False),retirement=retirement)

    def test_retiring_remains_fully_charged_and_only_bound_settlement_is_zero(self):
        retiring=self.record()
        self.assertEqual(runtime.material_source_charge(retiring),retiring['capture']['reserved_material_bytes'])
        self.assertEqual(runtime.material_source_charge(self.record('settled')),0)
        original=self.record();original['retirement']=None;original['restore']['material_use']=True
        self.assertEqual(runtime.material_source_charge(original),original['capture']['reserved_material_bytes'])

    def test_unknown_missing_partial_or_forged_settlement_is_rejected(self):
        import copy
        original=self.record('settled')
        mutations=[]
        for name in ('retirement','capture','restore'):
            changed=copy.deepcopy(original);del changed[name];mutations.append(changed)
        for name in ('plan','settlement','state'):
            changed=copy.deepcopy(original);del changed['retirement'][name];mutations.append(changed)
        changed=copy.deepcopy(original);changed['retirement']['state']='unknown';mutations.append(changed)
        for name in ('work_binding','plan_binding','released_material_bytes'):
            changed=copy.deepcopy(original)
            changed['retirement']['settlement'][name]=0 if name=='released_material_bytes' else [0]*32
            mutations.append(changed)
        changed=copy.deepcopy(original);changed['retirement']['plan']['bytes'][0]^=1;mutations.append(changed)
        changed=copy.deepcopy(original);changed['capture']['full']['disk_bytes']+=1;mutations.append(changed)
        changed=copy.deepcopy(original);changed['restore']['material_use']=True;mutations.append(changed)
        changed=copy.deepcopy(original);changed['retirement']['plan']['quiescence']['worker_drained']='Unconfirmed';mutations.append(changed)
        changed=copy.deepcopy(original);changed['retirement']['plan']['quiescence']['sandbox']+=1;mutations.append(changed)
        for changed in mutations:
            with self.subTest(changed=changed),self.assertRaises(ValueError):
                runtime.material_source_charge(changed)

    def test_retirement_never_creates_a_free_missing_or_zero_capture(self):
        for capture in (None,dict(reserved_material_bytes=0)):
            record=self.record('settled');record['capture']=capture
            with self.subTest(capture=capture),self.assertRaises(ValueError):
                runtime.material_source_charge(record)
