"""Controlled checks for expiry observations, not an alternative lifecycle model."""
import copy
import json
import sqlite3
import tempfile
from pathlib import Path
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock,patch

SYSTEM=Path(__file__).resolve().parents[2]/'tests/system'
sys.path.insert(0,str(SYSTEM))
import lifecycle_unload_expiry as expiry


class ExpiryObservationTests(unittest.TestCase):
    def fixture(self):
        key=dict(deployment='deployment',owner='1',window=1,sequence=3)
        resume=dict(session='session',key=key,accepted_version=3)
        lifetime=dict(started_at_ms=1000,end_at_ms=76000,max_end_at_ms=601000,startup_deadline_ms=301000)
        binding=dict(session='session',node={'node':'original'},protocol_version=2,
                     key=key,expected={'state':'version','version':2})
        prior=dict(binding=binding,accepted_version=3,center={'center':'original'},
                   retention={'slot':1},operation={'kind':'resume'},acceptance_time={'kind':'sampled','at_ms':2000})
        terminal=copy.deepcopy(prior);terminal.update(accepted_version=4,operation={'kind':'destroy'},acceptance_time={'kind':'unsampled'})
        terminal['binding'].update(key=dict(key,window=2,sequence=7),expected={'state':'version','version':3})
        row=dict(create=dict(binding={'session':'session'},timeout_ms=75000),
                 operations=[dict(operation={'kind':'create'}),dict(operation={'kind':'unload'}),prior,terminal],
                 terminating=True,released=False,last_running=dict(lifetime,observed_at_ms=2000))
        return {'sandboxes':[row]},resume,lifetime

    def test_only_observed_internal_terminal_key_is_returned(self):
        audit,resume,lifetime=self.fixture()
        accepted=expiry.expiry_acceptance(audit,'session',resume,lifetime,76000)
        self.assertEqual(accepted,dict(session='session',key=dict(resume['key'],window=2,sequence=7),accepted_version=4))
        self.assertEqual(resume['key']['window'],1)
        self.assertEqual(audit['sandboxes'][0]['operations'][-1]['acceptance_time'],{'kind':'unsampled'})

    def test_clock_or_pending_history_cannot_be_promoted_to_expiry(self):
        audit,resume,lifetime=self.fixture()
        with self.assertRaises(ValueError):expiry.expiry_acceptance(audit,'session',resume,lifetime,75999)
        row=audit['sandboxes'][0];row['operations'].pop();row['terminating']=False
        self.assertIsNone(expiry.expiry_acceptance(audit,'session',resume,lifetime,76000))

    def test_foreign_or_public_destroy_changed_lifetime_and_early_release_fail(self):
        original,resume,lifetime=self.fixture()
        mutations=[
            lambda r:r['operations'][-1]['binding']['key'].update(window=1),
            lambda r:r['operations'][-1]['binding']['key'].update(owner='2'),
            lambda r:r['operations'][-1]['binding']['expected'].update(version=2),
            lambda r:r['operations'][-1].update(accepted_version=5),
            lambda r:r['last_running'].update(end_at_ms=76001),
            lambda r:r.update(released=True),
            lambda r:r['operations'][-1].update(acceptance_time={'kind':'sampled','at_ms':76000}),
        ]
        for mutate in mutations:
            audit=copy.deepcopy(original);mutate(audit['sandboxes'][0])
            with self.subTest(mutate=mutate),self.assertRaises(ValueError):
                expiry.expiry_acceptance(audit,'session',resume,lifetime,76000)

    def test_explicit_destroy_is_rejected_before_any_background_observation(self):
        runtime=Mock();client=SimpleNamespace(destroy_request={'window':1})
        with self.assertRaises(ValueError):
            expiry.observe_center_expiry(runtime,client,{}, {},time.monotonic()+1,host_delivered=True)
        self.assertEqual(runtime.mock_calls,[])

    def test_wait_for_real_time_uses_original_deadline(self):
        with patch.object(expiry,'now_ms',side_effect=[75900,76000]),patch.object(expiry.time,'sleep') as sleep:
            expiry.wait_clock({'end_at_ms':76000},time.monotonic()+1)
        sleep.assert_called_once_with(.1)
        with patch.object(expiry,'now_ms',return_value=76000),self.assertRaises(ValueError):
            expiry.require_before_expiry({'end_at_ms':76000})

    def test_cleaned_wait_does_not_send_or_time_a_destroy(self):
        client=Mock();client.owner_key='key';client.path.return_value='/session';client.current_acceptance={'original':'expiry'}
        client.call.side_effect=[(200,json.dumps({'phase':'terminating'}).encode()),(200,json.dumps({'phase':'cleaned'}).encode())]
        with patch.object(expiry.time,'sleep'):
            self.assertEqual(expiry.wait_expired_cleaned(client,time.monotonic()+1),{'phase':'cleaned'})
        self.assertTrue(all(c.args[0]=='GET' for c in client.call.call_args_list))
        client.destroy.assert_not_called();client.operation_sent.assert_not_called()

    def test_expired_wait_rejects_a_revived_running_state(self):
        client=Mock();client.owner_key='key';client.path.return_value='/session'
        client.call.return_value=(200,b'{"phase":"running"}')
        with self.assertRaises(ValueError):expiry.wait_expired_cleaned(client,time.monotonic()+1)


class PrepareExpiryBarrierTests(unittest.TestCase):
    def test_native_deadline_read_requires_original_operation_and_terminal_compensation(self):
        probe,snapshot=self.fixture()
        with tempfile.TemporaryDirectory() as directory:
            database=Path(directory)/'db'
            probe.runtime.services.native_pools=[{'database':str(database),'namespace':'ns'}]
            probe.runtime.services.host_configs=[{'native':{'maintenance':{'capacity':{'max_sql_value_bytes':16384}}}}]
            probe.prior={'local':{'sandbox':1}}
            probe.action['identity'].update(owner='owner',operation='op-ns-1-3')
            with sqlite3.connect(database) as conn:
                conn.execute('CREATE TABLE sandboxes(sandbox_id TEXT,state_blob TEXT,owner_id TEXT)')
                conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',('sandbox',json.dumps(snapshot['native']),'owner'))
            with patch.object(expiry,'private_path',return_value=database):
                self.assertEqual(probe._wait_native_expiry(time.monotonic()+1)['operation']['id'],3)
                for changes in ({'id':4},{'kind':'Kill'},{'status':'Pending'}):
                    state=copy.deepcopy(snapshot['native']);state['current_operation'].update(changes)
                    with sqlite3.connect(database) as conn:
                        conn.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(state),))
                    with self.subTest(changes=changes),self.assertRaises(ValueError):
                        probe._wait_native_expiry(time.monotonic()+1)

    def fixture(self):
        probe=expiry.ExpiryPrepareProbe.__new__(expiry.ExpiryPrepareProbe)
        probe.action=dict(identity=dict(epoch=3,generation=2,resource='vm'),
                          command={'logical':{'command':{'PrepareVm':{'vm':2}}}})
        row=(probe.action,'dispatched',None)
        probe.config={'sandbox':'sandbox'}
        probe.runtime=Mock();probe.runtime.services.native_pools=[{'node_state_dir':'/unused'}]
        probe.runtime._materials.return_value={'materials':[1,2,3]}
        probe.restore_record=dict(work='work',capture={'full':'full'},retirement=None,
                                  restore=dict(intent='intent',activation=None,material_use=True))
        probe.restore_entry=probe.restore_record['restore']
        snapshot=dict(original={'local_operation':3},terminal={'local_operation':3},
                      local={'sandbox':1},resume_actions=[row],terminal_actions=[row],
                      sources=[copy.deepcopy(probe.restore_record)],
                      native=dict(epoch=3,generation=2,terminal=True,phase='Revoking',
                                  desired='Closed',effective='Closed',current_vm=None,net_slot=None,
                                  current_operation=dict(id=3,epoch=3,kind='Resume',
                                                         status={'Compensating':'DeadlineExceeded'})))
        return probe,snapshot

    def test_shared_operation_is_counted_once_but_real_duplicates_and_activation_fail(self):
        probe,snapshot=self.fixture()
        self.assertEqual(probe._terminal_inventory(snapshot),snapshot['resume_actions'][0])
        duplicate=copy.deepcopy(snapshot)
        duplicate['resume_actions']*=2;duplicate['terminal_actions']*=2
        with self.assertRaises(ValueError):probe._terminal_inventory(duplicate)
        inconsistent=copy.deepcopy(snapshot);inconsistent['terminal_actions']=[]
        with self.assertRaises(ValueError):probe._terminal_inventory(inconsistent)
        activation=(dict(command={'logical':{'command':{'ActivateVm':{}}}}),'ready',None)
        snapshot['resume_actions'].append(activation);snapshot['terminal_actions'].append(activation)
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)

    def test_only_closed_exact_deadline_compensation_allows_fault_withdrawal(self):
        probe,snapshot=self.fixture()
        with patch.object(expiry,'original_json',return_value=dict(epoch=3,terminal=False)), \
                patch.object(expiry,'material_source_charge',return_value=42):
            self.assertEqual(probe._terminal_pause_barrier(snapshot)['kind'],'same-resume-deadline-compensation')
            for changes in ({'effective':'Open'},{'epoch':4},{'terminal':False},
                            {'current_operation':dict(id=3,epoch=3,kind='Resume',status='Running')}):
                bad=copy.deepcopy(snapshot);bad['native'].update(changes)
                with self.subTest(changes=changes),self.assertRaises(ValueError):probe._terminal_pause_barrier(bad)
            bad=copy.deepcopy(snapshot);bad['sources'][0]['restore']['material_use']=False
            with self.assertRaises(ValueError):probe._terminal_pause_barrier(bad)
        with patch.object(expiry,'original_json',return_value=dict(epoch=2,terminal=False)),self.assertRaises(ValueError):
            probe._terminal_pause_barrier(snapshot)

    def test_cleanup_cut_requires_real_matching_terminal_fence_receipt_and_file(self):
        probe,snapshot=self.fixture()
        def receipt(action,kind,value):
            return (action,'receipted',dict(identity=action['identity'],outcome=dict(
                status='observed_success',details=dict(resource=action['identity']['resource'],
                command=action['command']['logical']['command'],result={kind:value}))))
        prepared=receipt(probe.action,'Prepared',dict(pid=1,start_ticks=2,boot_id='boot'))
        fence=dict(identity=dict(epoch=3,resource='fence'),command={'logical':{'command':{
            'InstallFence':dict(sandbox=1,epoch=3,terminal=True)}}})
        expected=dict(epoch=3,terminal=True)
        snapshot['resume_actions']=snapshot['terminal_actions']=[prepared]
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)
        snapshot['resume_actions'].append((fence,'dispatched',None))
        with self.assertRaises(ValueError):probe._terminal_inventory(snapshot)
        snapshot['resume_actions'][-1]=receipt(fence,'Fence',expected)
        with patch.object(expiry,'original_json',return_value=expected):
            self.assertEqual(probe._terminal_inventory(snapshot),prepared)
        with patch.object(expiry,'original_json',return_value=dict(epoch=3,terminal=False)),self.assertRaises(ValueError):
            probe._terminal_inventory(snapshot)


class RunningExpiryStopAttributionTests(unittest.TestCase):
    """Real readonly SQLite; process/material/root-fence facts are controlled."""
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name);self.database=self.root/'native.sqlite3'
        self.native_id='sb-expiry-1';self.operation_id='op-expiry-1-4'
        self.action=self.stop_action(sandbox=1,operation=4,generation=2,vm=2,epoch=5)
        state=dict(terminal=True,epoch=5,current_operation=dict(id=4,kind='Kill'))
        with sqlite3.connect(self.database) as conn:
            conn.executescript('CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,state_blob BLOB);'
                'CREATE TABLE outbox(id INTEGER PRIMARY KEY,sandbox_id TEXT NOT NULL,owner_id TEXT NOT NULL,'
                'operation_id TEXT NOT NULL,logical_action_id TEXT NOT NULL,attempt INTEGER NOT NULL CHECK(attempt>=0),'
                'action_json BLOB NOT NULL,dispatch_state TEXT NOT NULL CHECK(dispatch_state IN '
                "('ready','dispatched','receipted')),receipt_json BLOB,"
                'UNIQUE(sandbox_id,logical_action_id,attempt),'
                "CHECK((dispatch_state='receipted')=(receipt_json IS NOT NULL)));")
            conn.execute('INSERT INTO sandboxes VALUES(?,?)',(self.native_id,json.dumps(state).encode()))
        self.database.chmod(0o600)
        self.insert_action(self.action)
        self.material=dict(materials=[{'original':n} for n in range(3)],manifest={'original':'manifest'},
                           completion={'original':'completion'},material_paths=['state','memory','disk'])
        services=SimpleNamespace(native_pools=[dict(database=str(self.database),namespace='expiry',
            node_state_dir=str(self.root))],host_configs=[dict(native=dict(maintenance=dict(
            capacity=dict(max_sql_value_bytes=16384))))],processes=Mock())
        self.probe=expiry.RunningExpiryProbe.__new__(expiry.RunningExpiryProbe)
        self.probe.runtime=SimpleNamespace(services=services,_materials=Mock(return_value=copy.deepcopy(self.material)))
        self.probe.client=SimpleNamespace(session='original-session')
        self.probe.running=dict(source=dict(resource_id='vm-expiry-2',pid=41,start_time_ticks=7,boot_id='boot'),
            local=dict(sandbox=1),context=dict(binding=dict(generation=2,vm=2)),
            running_backing=copy.deepcopy(self.material))
        self.probe.before=dict(capture=dict(full={'original':'source'}),restore=dict(activation=dict(
            identity=dict(owner='tenant-1',protocol_version=1)),material_use=True))
        self.probe._source=Mock(side_effect=lambda _:copy.deepcopy(self.probe.before))
        self.probe.lifetime=dict(end_at_ms=1);self.probe.pidfd=41
        self.probe.host=Mock();self.probe.host.wait.return_value=87
        for patcher in [patch.object(expiry,'pidfd_exited',return_value=False),
                        patch.object(expiry,'original_process',return_value=True),
                        patch.object(expiry,'material_source_charge',return_value=100),
                        patch.object(expiry,'original_json',return_value=dict(epoch=5,terminal=True))]:
            patcher.start();self.addCleanup(patcher.stop)

    @staticmethod
    def stop_action(*,sandbox,operation,generation,vm,epoch):
        native_id='sb-expiry-'+str(sandbox);opid='op-expiry-'+str(sandbox)+'-'+str(operation)
        return dict(identity=dict(protocol_version=1,owner='tenant-'+str(sandbox),sandbox=native_id,
            operation=opid,epoch=epoch,generation=generation,logical_action='stop-'+opid,attempt=0,
            resource=dict(kind='vm',id='vm-expiry-'+str(vm))),command=dict(logical=dict(command=dict(
            StopVm=dict(sandbox=sandbox,generation=generation,vm=vm,epoch=epoch))),attempt=0))

    def insert_action(self,action,state='dispatched'):
        identity=action['identity']
        # Historical receipt content is not used as a gate-arrival fact.
        receipt=json.dumps(dict(identity=identity,outcome=dict(status='observed_success',details=dict(
            resource=identity['resource'],command=action['command']['logical']['command'],
            result=dict(Stopped=dict(pid=51,start_ticks=17)))))).encode() if state=='receipted' else None
        with sqlite3.connect(self.database) as conn:
            row=conn.execute('INSERT INTO outbox(sandbox_id,owner_id,operation_id,logical_action_id,attempt,'
                'action_json,dispatch_state,receipt_json) VALUES(?,?,?,?,?,?,?,?)',
                (identity['sandbox'],identity['owner'],identity['operation'],identity['logical_action'],
                 identity['attempt'],json.dumps(action).encode(),state,receipt))
            return row.lastrowid

    def remove_action(self,row):
        with sqlite3.connect(self.database) as conn:conn.execute('DELETE FROM outbox WHERE id=?',(row,))

    def observe(self):
        return self.probe.held_after_expiry(time.monotonic()+10)

    def test_one_original_dispatched_stop_certifies_the_held_cut_without_writes(self):
        before=self.database.read_bytes()
        result=self.observe()
        self.assertTrue(result['original_stop_dispatched_without_receipt'])
        self.assertTrue(result['original_vmm_live']);self.assertFalse(result['physical_exit'])
        self.assertEqual(result['material_charge'],100)
        self.assertEqual(self.database.read_bytes(),before)
        self.probe.runtime.services.processes.assert_not_called()

    def test_another_dispatched_stop_cannot_be_attributed_to_the_original_exit87(self):
        actions=[self.stop_action(sandbox=2,operation=3,generation=1,vm=3,epoch=4),
                 self.stop_action(sandbox=1,operation=3,generation=1,vm=1,epoch=4)]
        for action in actions:
            row=self.insert_action(action)
            try:
                with self.subTest(identity=action['identity']),self.assertRaises(ValueError):self.observe()
                self.probe.runtime.services.processes.assert_not_called()
            finally:self.remove_action(row)

    def test_unclaimed_historical_or_nonstop_work_does_not_create_a_false_ambiguity(self):
        other=self.stop_action(sandbox=2,operation=3,generation=1,vm=3,epoch=4)
        for state in ('ready','receipted'):
            row=self.insert_action(other,state)
            try:
                with self.subTest(state=state):self.assertTrue(self.observe()['original_vmm_live'])
            finally:self.remove_action(row)
        freeze=copy.deepcopy(other)
        freeze['command']['logical']['command']={'FreezeVm':freeze['command']['logical']['command']['StopVm']}
        row=self.insert_action(freeze)
        try:self.assertTrue(self.observe()['original_vmm_live'])
        finally:self.remove_action(row)

    def replace_original_action(self,action):
        with sqlite3.connect(self.database) as conn:
            conn.execute('UPDATE outbox SET action_json=? WHERE sandbox_id=? AND operation_id=?',
                (json.dumps(action).encode(),self.native_id,self.operation_id))

    def test_full_original_stop_parameters_preserve_integer_json_types(self):
        for field,value in [('sandbox',True),('generation',2.0),('vm',2.0),('epoch',5.0)]:
            action=copy.deepcopy(self.action);action['command']['logical']['command']['StopVm'][field]=value
            self.replace_original_action(action)
            with self.subTest(field=field,value=value),self.assertRaises(ValueError):self.observe()
        self.replace_original_action(self.action)

    def test_original_wire_identity_preserves_protocol_generation_and_epoch_types(self):
        for field,value in [('protocol_version',True),('protocol_version',1.0),('generation',2.0),('epoch',5.0)]:
            action=copy.deepcopy(self.action);action['identity'][field]=value
            self.replace_original_action(action)
            with self.subTest(field=field,value=value),self.assertRaises(ValueError):self.observe()
        self.replace_original_action(self.action)

    def test_protected_terminal_fence_preserves_epoch_and_boolean_types(self):
        for fence in [dict(epoch=5.0,terminal=True),dict(epoch=5,terminal=1),dict(epoch=5,terminal=1.0)]:
            with self.subTest(fence=fence),patch.object(expiry,'original_json',return_value=fence), \
                    self.assertRaises(ValueError):self.observe()


if __name__=='__main__':unittest.main()
