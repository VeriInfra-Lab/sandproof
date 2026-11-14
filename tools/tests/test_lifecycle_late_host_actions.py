"""Finite scene wiring, real SQLite capture and fail-closed observer composition."""
import copy
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_late_host_actions as late
import lifecycle_late_material_receipt as receipt
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
from tools.tests.test_lifecycle_late_material_receipt import LateMaterialFixture


def fixture():
    context = dict(position=0, expected_previous=1, binding=dict(owner=1,sandbox=1,generation=2,vm=2))
    prior = dict(session='session-A',context=context,local=dict(owner=1,sandbox=1),
        source=dict(pid=41,start_time_ticks=11))
    node = dict(deployment='deployment',node='node')
    resume = dict(session='session-A',accepted_version=3,
        key=dict(deployment='deployment',owner='1',window=1,sequence=3))
    intent = dict(accepted_version=3,center=dict(center='original'),retention=dict(window=1),
        request=dict(binding=dict(protocol_version=4,node=node,session='session-A',key=resume['key'],
            expected=dict(state='version',version=2)),operation=dict(kind='resume')))
    original = dict(intent=copy.deepcopy(intent),local_operation=3)
    ending = copy.deepcopy(intent); ending['accepted_version']=4
    ending['request']['binding']['key']=dict(resume['key'],sequence=4)
    ending['request']['binding']['expected']['version']=3
    ending['request']['operation']=dict(kind='destroy')
    accepted = dict(session='session-A',accepted_version=4,key=ending['request']['binding']['key'])
    entry = dict(intent=ending,local_operation=4)
    inbox = dict(sessions=[dict(id='session-A',current_version=4,terminal=True,
        local=prior['local'],entries=[original,entry])])
    resource = dict(kind='sandbox',id='sb-original-1')
    def action(kind):
        command = dict(sandbox=1,generation=2,vm=2)
        if kind == 'StopVm': command['epoch']=5
        return dict(identity=dict(owner='tenant-1',sandbox='sb-original-1',operation='op-original-1-4',
            protocol_version=4,generation=2,epoch=5,resource=resource,attempt=1),
            command=dict(attempt=1,logical=dict(command={kind:command})))
    stop, cleanup = action('StopVm'),action('CleanupVm')
    result = dict(identity=copy.deepcopy(stop['identity']),outcome=dict(status='observed_success',
        details=dict(resource=resource,command=copy.deepcopy(stop['command']['logical']['command']),
            result=dict(Stopped=dict(pid=41,start_ticks=11)))))
    rows = [(stop,'receipted',result),(cleanup,'dispatched',None)]
    return prior,resume,original,accepted,inbox,rows,resource


class OriginalLateActionsTests(unittest.TestCase):
    def test_capture_uses_complete_actual_actions_and_receipts(self):
        prior,_,_,_,_,rows,resource = fixture()
        selected = late.original_actions(rows,prior,{'epoch':5},resource)
        self.assertEqual(selected,[rows[0][0],rows[1][0]])
        selected[0]['identity']['attempt']=8
        self.assertEqual(rows[0][0]['identity']['attempt'],1)
        for fault in ('missing','duplicate','vm','generation','resource','epoch','command',
                      'receipt-pid','receipt-identity','receipt-command','unknown','cleanup-receipted'):
            bad=copy.deepcopy(rows)
            if fault=='missing': bad.pop()
            elif fault=='duplicate': bad.append(copy.deepcopy(bad[0]))
            elif fault in ('vm','generation'): bad[1][0]['command']['logical']['command']['CleanupVm'][fault]+=1
            elif fault in ('resource','epoch'): bad[1][0]['identity'][fault]='foreign'
            elif fault=='command': bad[0][0]['command']['logical']['command']['StopVm']['epoch']=8
            elif fault=='receipt-pid': bad[0][2]['outcome']['details']['result']['Stopped']['pid']=42
            elif fault=='receipt-identity': bad[0][2]['identity']['attempt']=2
            elif fault=='receipt-command': bad[0][2]['outcome']['details']['command']={}
            elif fault=='unknown': bad[0][2]['outcome']['status']='unknown'
            else: bad[1]=(bad[1][0],'receipted',{})
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                late.original_actions(bad,prior,{'epoch':5},resource)

    def test_public_native_mapping_and_full_destroy_intent_cannot_drift(self):
        prior,resume,original,accepted,inbox,rows,resource=fixture()
        node=original['intent']['request']['binding']['node']
        entry,local=late.accepted_entry(inbox,accepted,prior,node,terminal=True)
        self.assertEqual(local,prior['local'])
        for fault in ('current-version','terminal','local','key','session','node','operation-missing'):
            bad=copy.deepcopy(inbox); current=bad['sessions'][0]
            if fault=='current-version': current['current_version']+=1
            elif fault=='terminal': current['terminal']=False
            elif fault=='local': current['local']['sandbox']=2
            elif fault=='key': current['entries'][-1]['intent']['request']['binding']['key']['sequence']+=1
            elif fault in ('session','node'): current['entries'][-1]['intent']['request']['binding'][fault]='foreign'
            else: current['entries'][-1]['local_operation']=None
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                late.accepted_entry(bad,accepted,prior,node,terminal=True)
        client=NS(session=prior['session'],current_acceptance=resume,
            destroy_request=dict(window=1,sequence=4,expected_version=3))
        runner=NS(services=NS(native_pools=[{}],host_configs=[{}],processes=Mock()))
        for fault in ('none','center','retention','expected','protocol','operation','local-operation',
                      'host-exit','host-replaced','old-live'):
            probe=late.LateHostActionsProbe(runner,client,prior,resume,{},time.monotonic()+5)
            probe.entry=original;probe.resource=resource;probe.host=Mock()
            probe.host.poll.return_value=86
            runner.services.processes.children={'host-0':probe.host}
            bad=copy.deepcopy(entry); badrows=copy.deepcopy(rows)
            if fault in ('center','retention'): bad['intent'][fault]={'foreign':True}
            elif fault in ('expected','protocol'):
                bad['intent']['request']['binding']['expected' if fault=='expected' else 'protocol_version']=0
            elif fault=='operation': bad['intent']['request']['operation']={'kind':'unload'}
            elif fault=='local-operation': badrows[1][0]['identity']['epoch']=7
            elif fault=='host-exit':probe.host.poll.return_value=1
            elif fault=='host-replaced':runner.services.processes.children['host-0']=Mock()
            probe._read=Mock(return_value=(bad,{'epoch':5},badrows))
            client.current_acceptance=accepted
            with self.subTest(fault=fault),patch.object(late,'original_process',return_value=fault=='old-live'):
                if fault=='none': self.assertEqual(probe.capture(accepted)['actual_commands'],['StopVm','CleanupVm'])
                else:
                    with self.assertRaises(ValueError):probe.capture(accepted)
            client.current_acceptance=resume

    def test_approved_helper_is_checked_before_services_can_start(self):
        for extra in ({},{'stale_probe':'/other/stale_host_actions','stale_probe_sha256':'a'*64},
                      {'stale_probe':'/fixed/release/examples/stale_host_actions','stale_probe_sha256':'bad'}):
            args=NS(scenario='slot-reuse-late-stop-delete',scenario_timeout=60,
                activation_gate_timeout_ms=60000,host='/fixed/release/host',**extra)
            with self.subTest(extra=extra),patch.object(runtime,'prepare') as prepare,self.assertRaises(ValueError):
                scene.exercise(args)
            prepare.assert_not_called()


class LateActionsSqlTests(unittest.TestCase):
    def test_original_bounded_sql_capture_preserves_readonly_database(self):
        prior,resume,_,accepted,inbox,rows,_=fixture()
        with tempfile.TemporaryDirectory() as directory:
            database=Path(directory)/'db'
            with sqlite3.connect(database) as conn:
                conn.executescript('CREATE TABLE controller_lifecycle_inbox(id,state_json,reserve_bytes,sha256);'
                    'CREATE TABLE sandboxes(sandbox_id,state_blob);'
                    'CREATE TABLE outbox(id,sandbox_id,operation_id,action_json,dispatch_state,receipt_json);')
                raw=json.dumps(inbox).encode()
                conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',
                    (raw,16384,hashlib.sha256(raw).digest()))
                conn.execute('INSERT INTO sandboxes VALUES(?,?)',('sb-original-1',json.dumps(dict(owner=1,generation=2,
                    epoch=5,terminal=True,current_operation=dict(kind='Kill',id=4,epoch=5)))))
                for index,(action,state,result) in enumerate(rows):
                    conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?)',(index,'sb-original-1','op-original-1-4',
                        json.dumps(action),state,None if result is None else json.dumps(result)))
            runner=NS(services=NS(native_pools=[dict(database=str(database),namespace='original')],
                host_configs=[dict(inbox=dict(node=inbox['sessions'][0]['entries'][0]['intent']['request']['binding']['node']),
                    native=dict(maintenance=dict(capacity=dict(max_sql_value_bytes=16384,max_sql_rows=10))))]))
            probe=late.LateHostActionsProbe(runner,NS(session=prior['session'],current_acceptance=resume),
                prior,resume,{},time.monotonic()+5)
            probe._configuration=Mock()
            before=database.read_bytes()
            with patch.object(late,'private_path',side_effect=Path):
                self.assertEqual(probe._read(accepted,terminal=True)[2],rows)
                self.assertEqual(database.read_bytes(),before)
                state=dict(owner=1,generation=2,epoch=5,terminal=True,current_operation=dict(kind='Kill',id=4,epoch=5))
                for fault in ('checksum','bound','attempt','owner','operation','protocol_version','generation','native-kill'):
                    with sqlite3.connect(database) as conn:
                        conn.execute('UPDATE controller_lifecycle_inbox SET sha256=?,reserve_bytes=16384',
                            (hashlib.sha256(raw).digest(),))
                        conn.execute('UPDATE outbox SET action_json=? WHERE id=0',(json.dumps(rows[0][0]),))
                        conn.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(state),))
                        if fault=='checksum':conn.execute('UPDATE controller_lifecycle_inbox SET sha256=zeroblob(32)')
                        elif fault=='bound':conn.execute('UPDATE controller_lifecycle_inbox SET reserve_bytes=1')
                        elif fault=='native-kill':
                            bad=copy.deepcopy(state);bad['current_operation']['id']=5
                            conn.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(bad),))
                        else:
                            bad=copy.deepcopy(rows[0][0]);bad['identity'][fault]='foreign'
                            conn.execute('UPDATE outbox SET action_json=? WHERE id=0',(json.dumps(bad),))
                    with self.subTest(fault=fault),self.assertRaises(ValueError):probe._read(accepted,terminal=True)


class SlotOccupantTests(LateMaterialFixture):
    def test_generic_observer_needs_no_retiring_old_source_and_freezes_all_other_facts(self):
        self.records.pop(0)  # Unlike the receipt observer, A may be wholly retired.
        observer=receipt.SlotOccupantObserver(self.runtime,occupant=self.occupant,deadline=time.monotonic()+5)
        with patch.object(observer,'_physical',side_effect=lambda _:copy.deepcopy(self.physical)):
            observer.freeze();observer.verify()
            with self.assertRaises(ValueError):observer.freeze()
            self.records[0]['capture']['reserved_material_bytes']+=1
            with self.assertRaises(ValueError):observer.verify()
            self.records[0]['capture']['reserved_material_bytes']-=1
            self.update('UPDATE controller_data_credentials SET digest=zeroblob(32)')
            with self.assertRaises(ValueError):observer.verify()


class LateActionsReplayTests(unittest.TestCase):
    def test_refusal_requires_independent_resources_and_actual_guest_callback(self):
        prior,resume,_,_,_,rows,_=fixture()
        occupant=dict(session='B',context=dict(position=0,expected_previous=2,
            binding=dict(owner=2,sandbox=2,generation=1,vm=3)))
        stamp=dict(path='/fixed/helper',sha256='a'*64,expected_sha256='a'*64)
        valid=dict(version=1,context_rejected=True,release_rejected=True,all_actions_rejected=True,checked_actions=2)
        for fault in ('none','helper-failed','incomplete','observed-success','metadata','kernel','sql','guest','reuse','helper-changed'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                processes=Mock();runner=NS(root=Path(directory),services=NS(processes=processes,
                    wait_ready=Mock(),native_pools=[dict(node_state_dir='original-state',chroot_base='original-jail')],
                    host_configs=[{}]))
                probe=late.LateHostActionsProbe(runner,NS(session='session-A',current_acceptance=resume),
                    prior,resume,stamp,time.monotonic()+10)
                probe._configuration=Mock();probe.recovered=True;probe.actions=[row[0] for row in rows]
                observer=Mock();observer.verify.return_value={}
                if fault=='sql':observer.verify.side_effect=ValueError('original SQL changed')
                stack.enter_context(patch.object(late,'SlotOccupantObserver',return_value=observer))
                stack.enter_context(patch.object(late,'private_path',side_effect=lambda p,**_:Path(p)))
                stack.enter_context(patch.object(late,'node_configuration',return_value={'original':'config'}))
                stack.enter_context(patch.object(late,'artifact_fingerprint',return_value=
                    dict(stamp,sha256='b'*64) if fault=='helper-changed' else stamp))
                tree=stack.enter_context(patch.object(late,'tree_records',side_effect=[{'original':1},
                    {'original':2 if fault=='metadata' else 1}]))
                stack.enter_context(patch.object(late,'live_identity',side_effect=[{'pid':42},{'pid':43 if fault=='kernel' else 42}]))
                result=copy.deepcopy(valid)
                if fault=='incomplete':result['checked_actions']=1
                if fault=='observed-success':result['all_actions_rejected']=False
                command=stack.enter_context(patch.object(late,'bounded_json_command',return_value=result,
                    side_effect=ValueError('helper initialization failed') if fault=='helper-failed' else None))
                guest=Mock(return_value={'original':'guest'},side_effect=ValueError('guest changed') if fault=='guest' else None)
                current=copy.deepcopy(occupant)
                if fault=='reuse':current['context']['expected_previous']=1
                if fault=='none':
                    self.assertTrue(probe.replay(current,guest)['same_kernel_identity'])
                    guest.assert_called_once();self.assertEqual(observer.verify.call_count,2)
                    self.assertEqual(command.call_args.args[0][0],stamp['path'])
                    self.assertEqual(json.loads((Path(directory)/'late-stop-delete-actions.json').read_bytes())['actions'],probe.actions)
                    self.assertEqual(tree.call_count,2)
                    with self.assertRaises(ValueError):probe.replay(current,guest)
                else:
                    with self.assertRaises((ValueError,AssertionError)):probe.replay(current,guest)
                    if fault not in ('guest',):guest.assert_not_called()
                    if fault=='reuse':command.assert_not_called();processes.stop.assert_not_called()
                    if fault not in ('guest','reuse'):processes.start.assert_not_called()

    def test_bounded_real_child_failures_are_not_refusal_evidence(self):
        env={'PATH':'/usr/bin:/bin','LANG':'C'}
        for code in ('import sys;sys.exit(1)', 'print("not-json")', 'print("x"*1025)',
                     'import time;time.sleep(2)', 'print("{\\"version\\":1,\\"version\\":1}")'):
            with self.subTest(code=code),self.assertRaises((ValueError,TimeoutError)):
                late.bounded_json_command([sys.executable,'-c',code],env,timeout=.1,max_bytes=1024)


class LateStopDeleteSceneTests(unittest.TestCase):
    def test_scene_waits_for_product_gc_before_other_owner_and_preserves_failures(self):
        for failed in (False,True):
            with self.subTest(failed=failed),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                events=[]; lifetime=dict(started_at_ms=1,end_at_ms=100,max_end_at_ms=200,startup_deadline_ms=20)
                clients={}
                for owner in ('1','2'):
                    client=Mock(spec=scene.PublicUnloadScenario)
                    client.owner=owner;client.session='session-'+owner
                    client.current_acceptance=dict(key={},accepted_version=1)
                    client.unload_acceptance=dict(key={},accepted_version=2)
                    client.concurrent_unload_replies=[client.unload_acceptance]*2
                    client.timings=[dict(sent_monotonic=time.monotonic())];client.timing_report.return_value=[]
                    client.failure_step='controlled';client.last_http_status=None
                    client.wait_running.return_value=dict(phase='running')
                    client.wait_unloaded.return_value=dict(phase='unloaded')
                    for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_resumed_workload'):
                        getattr(client,name).side_effect=lambda *_,o=owner,n=name,**__:events.append(n+'-'+o) or {}
                    clients[owner]=client
                def record(session,*_,**__):return dict(session=session,lifetime=lifetime,context=dict(position=0))
                runner=NS(root=Path(directory),owner_key='owner',foreign_key='foreign',
                    services=NS(sdk_url='http://127.0.0.1:1'),measurement_identity=lambda:{},start=Mock(),
                    create_parameters=lambda **_:{},record_running=Mock(side_effect=record),
                    unload_evidence=Mock(return_value=dict(facts=lifetime,original_position=dict(position=0),original_capture_responsibilities=1)),
                    restart_managers=Mock(return_value=dict(original_unload_operation={})),
                    destroy_evidence=Mock(side_effect=lambda session,_:events.append('gc-'+session) or {}),
                    settlement_evidence=Mock(side_effect=lambda _:events.append('zero-quota') or {}),finish=Mock(),
                    fail_resume_gate=Mock(return_value='preserved'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
                probe=Mock()
                for name in ('arm','capture','recover'):
                    getattr(probe,name).side_effect=lambda *_,n=name:events.append(n) or {}
                def replay(_,guest):
                    events.append('late-stop-delete')
                    if failed:raise ValueError('controlled stale action failure')
                    guest();return {}
                probe.replay.side_effect=replay
                stack.enter_context(patch.object(late,'LateHostActionsProbe',return_value=probe))
                stack.enter_context(patch.object(late,'approved_probe',return_value={}))
                stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
                stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
                stack.enter_context(patch.object(scene,'check_center_configuration',return_value=dict(deployment='original')))
                args=NS(scenario='slot-reuse-late-stop-delete',scenario_timeout=60,activation_gate_timeout_ms=60000)
                if failed:
                    with self.assertRaises(ValueError):scene.exercise(args)
                    report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                    self.assertFalse(report['complete']);self.assertEqual(len(report['known_sessions']),2)
                    self.assertNotIn('destroy-2',events);clients['2'].assert_running_workload.assert_not_called()
                else:
                    report=scene.exercise(args);self.assertTrue(report['complete'])
                    self.assertEqual(events.count('destroy-1'),1);self.assertEqual(events.count('destroy-2'),1)
                    self.assertLess(events.index('capture'),events.index('recover'))
                    self.assertLess(events.index('gc-session-1'),events.index('create-2'))
                    self.assertLess(events.index('zero-quota'),events.index('create-2'))
                    self.assertLess(events.index('late-stop-delete'),events.index('destroy-2'))
                    self.assertEqual(events[-1],'zero-quota');self.assertIn('E9',report['acceptance_cases'])
                self.assertLess(events.index('destroy-1'),events.index('late-stop-delete'))
                runner.finish.assert_called_once()

    def test_cli_registers_selector_and_exact_helper_inputs(self):
        result=subprocess.run([sys.executable,scene.__file__,'--help'],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('slot-reuse-late-stop-delete',result.stdout)
        self.assertIn('--stale-probe-sha256',result.stdout)
