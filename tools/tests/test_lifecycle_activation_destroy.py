"""E8 controlled providers, real SQLite/Unix framing, no product VM."""
import copy
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import socket
import sqlite3
import sys
import tempfile
import threading
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock,patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_activation_destroy as active
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
from tools.tests.test_lifecycle_resume_receipt_loss import fixture as resume_fixture


def fixture(root):
    f=resume_fixture();ns='0123456789abcdef';name='sb-'+ns+'-4';vm='vm-'+ns+'-9'
    source=dict(resource_id=vm,pid=123,start_time_ticks=456,boot_id='boot',launch_cpu_domain=[1,2])
    context=dict(position=1,expected_previous=None,binding=dict(owner=1,sandbox=4,generation=2,vm=9,network=10))
    local=dict(owner=1,sandbox=4,operation=3)
    def action(kind,epoch=3,operation=3):
        fields=dict(sandbox=4,generation=2,vm=9,epoch=epoch)
        if kind=='PrepareVm':fields.update(slot=10,source={'snapshot':1,'manifest':1})
        if kind=='InstallFence':fields=dict(sandbox=4,epoch=epoch,terminal=True)
        if kind=='CleanupVm':fields.pop('epoch')
        return dict(identity=dict(protocol_version=1,owner='tenant-1',sandbox=name,operation='op-'+ns+'-4-'+str(operation),
                    epoch=epoch,generation=2,attempt=0,resource=dict(kind='sandbox' if kind=='InstallFence' else 'vm',id=name if kind=='InstallFence' else vm)),
                    command=dict(attempt=0,logical=dict(command={kind:fields})))
    def receipt(a,kind,body):
        return dict(identity=copy.deepcopy(a['identity']),outcome=dict(status='observed_success',details=dict(
            resource=copy.deepcopy(a['identity']['resource']),command=copy.deepcopy(a['command']['logical']['command']),result={kind:body})))
    prepared=action('PrepareVm');activated=action('ActivateVm');stop=action('StopVm',4,4);fence=action('InstallFence',4,4);cleanup=action('CleanupVm',4,4)
    process=dict(pid=123,start_ticks=456,boot_id='boot')
    actions=[(prepared,'receipted',receipt(prepared,'Prepared',process)),(activated,'dispatched',None)]
    record=dict(work=dict(anchor=dict(local=dict(owner=1,sandbox=4,operation=2),request=f.unloaded['request'])),
        observation={'sample':1},capture=dict(full=dict(state_bytes=1,memory_bytes=2,disk_bytes=3),
            reserved_material_bytes=589836,completed=[7]*32,material_bytes=6,action={'original':'Seal'},worker={'original':'worker'},end=None),
        restore=dict(intent=f.request,local=local,generation=2,activation=activated,material_use=True),
        retirement=None,material_release=dict(producer=None,consumer=None))
    candidate=dict(claim=copy.deepcopy(record['restore']),context=context,source=source,physical={'original':'physical'},
        manifest=[7]*32,scope='one live original candidate and pin snapshot; not a complete API invocation trace')
    baseline=dict(nonce='1'*64,activation_nonce='2'*64,metadata_generation=1,counter=1,pid=73,start_time_ticks=74)
    observed=dict(session=f.acceptance['session'],operation=f.acceptance['key'],candidate={k:source[k] for k in ('resource_id','pid','start_time_ticks','boot_id')},
        namespace=dict(name=vm,device=1,inode=2),action_sha256=hashlib.sha256(json.dumps(activated,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
        ram_nonce_sha256=hashlib.sha256(baseline['nonce'].encode()).hexdigest(),activation_nonce_sha256=hashlib.sha256(b'fresh-RAM').hexdigest(),
        guest_pid=73,guest_start_time_ticks=74,counter=1,metadata_generation=2,public_authorization_proven=False)
    old=dict(intent=copy.deepcopy(f.request),local_operation=3,applied_deadline=None,previous_effective_end_at_ms=None,unload_resolution=None)
    destroy=copy.deepcopy(f.request);destroy['accepted_version']=4;destroy['request']['binding']['expected']['version']=3
    destroy['request']['binding']['key']['sequence']=4;destroy['request']['operation']={'kind':'destroy'}
    terminal=dict(old,intent=destroy,local_operation=4)
    accepted=dict(session=f.acceptance['session'],key=copy.deepcopy(destroy['request']['binding']['key']),accepted_version=4)
    native=dict(owner=1,terminal=True,phase='Revoking',generation=2,current_vm=9,net_slot=10,epoch=4,current_operation=dict(kind='Kill',id=4,epoch=4))
    snapshot=dict(original=old,terminal=terminal,local=local,native=native,sources=[record],resume_actions=actions,
        terminal_actions=[(fence,'receipted',receipt(fence,'Fence',dict(epoch=4,terminal=True))),(stop,'dispatched',None)],
        global_pending_stops=[stop],lifetime=f.lifetime)
    cut=copy.deepcopy(snapshot);cut['terminal_actions'][1]=(stop,'receipted',receipt(stop,'Stopped',dict(pid=123,start_ticks=456)))
    cut['terminal_actions'].append((cleanup,'dispatched',None));cut['global_pending_stops']=[]
    host=Mock(pid=777);host.poll.return_value=None;host.wait.return_value=86
    processes=NS(bindings={'host-0':dict(uid=os.getuid(),gid=os.getgid())},children={'host-0':host})
    settings=dict(namespace=ns,database=str(root/'state.db'),node_state_dir=str(root/'state'))
    config=dict(inbox=dict(node=f.node),native=dict(maintenance=dict(capacity=dict(max_sql_rows=64,max_sql_value_bytes=65536))))
    runner=NS(services=NS(native_pools=[settings],host_configs=[config],processes=processes),
        restore_gate=dict(host=host,session=f.acceptance['session'],command_sent=False,deadline=time.monotonic()+5),
        runs={f.acceptance['session']:dict(session=f.acceptance['session'],context=dict(position=0,binding={'generation':1}),
            local=dict(owner=1,sandbox=4,operation=1),source={'original':'old VMM'},lifetime=f.lifetime)},
        quota={'deployment_control':str(root)},_gate_files_unchanged=Mock(),_materials=Mock(return_value={'original':'material'}),
        finish_resume_gate=Mock(return_value=host),gc_complete=False)
    client=Mock(spec=scene.PublicUnloadScenario);client.session=f.acceptance['session'];client.current_acceptance=copy.deepcopy(f.acceptance)
    client.witness=baseline;client.destroy_request=dict(window=4,sequence=4,expected_version=3)
    p=active.ActivationDestroyProbe.__new__(active.ActivationDestroyProbe)
    p.runtime=runner;p.client=client;p.stage='restore-prepare-destroy';p.deadline=time.monotonic()+5;p.receipt=observed
    p.accepted=f.acceptance;p.terminal_accepted=None;p.restore_entry=old;p.action=activated;p.prepare_action=prepared
    p.candidate=candidate;p.initial_record=copy.deepcopy(record);p.material={'original':'material'}
    p.prior=dict(session=client.session,local=local,context=context,source=source,lifetime=f.lifetime)
    p.host=host;p.config={'sandbox':name};p.pidfd=999999;p.fd=None;p.stop_peer=None;p.stop_released=False;p.recovered=False;p.terminal_cut=None
    p._configuration=Mock();p.pause_requested=p.candidate_paused=False
    p.stop_path=root/'stop.sock'
    inbox=dict(sessions=[dict(id=client.session,current_version=4,terminal=True,local=local,entries=[old,terminal])])
    return NS(probe=p,runtime=runner,client=client,resume=f,source=source,record=record,candidate=candidate,receipt=observed,
        actions=actions,snapshot=snapshot,cut=cut,accepted_destroy=accepted,inbox=inbox,work_receipt=receipt,cleanup=cleanup,stop=stop)


class ActivationFactsTests(unittest.TestCase):
    def test_constructor_correlates_gate_full_source_and_latest_ram_before_opening_pidfd(self):
        for fault in ('none','source','cpu-domain','marker','old-RAM','nonce','counter','guest','generation','pin','position','receipt'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));observed=copy.deepcopy(f.receipt);candidate=copy.deepcopy(f.candidate)
                records=copy.deepcopy([f.record]);actions=copy.deepcopy(f.actions)
                if fault=='source':candidate['source']['pid']+=1
                if fault=='cpu-domain':candidate['source']['launch_cpu_domain']=[3]  # complete record is retained below
                if fault=='marker':candidate['claim']['activation']['identity']['epoch']+=1
                if fault=='old-RAM':observed['activation_nonce_sha256']=hashlib.sha256(f.client.witness['activation_nonce'].encode()).hexdigest()
                if fault=='nonce':observed['ram_nonce_sha256']='0'*64
                if fault=='counter':observed['counter']+=1
                if fault=='guest':observed['guest_pid']+=1
                if fault=='generation':observed['metadata_generation']+=1
                if fault=='pin':records[0]['restore']['material_use']=False
                if fault=='position':candidate['context']['position']=0
                if fault=='receipt':actions[1]=(actions[1][0],'receipted',{})
                stack.enter_context(patch.object(active.ActivationDestroyProbe,'_configuration'))
                stack.enter_context(patch.object(active.ActivationDestroyProbe,'_read',return_value=(f.probe.restore_entry,records,actions)))
                stack.enter_context(patch.object(active,'observe_candidate',return_value=candidate))
                stack.enter_context(patch.object(active,'original_process',return_value=True))
                opened=stack.enter_context(patch.object(active,'open_original_candidate_pidfd',return_value=999999))
                with self.subTest(fault=fault):
                    if fault in ('none','cpu-domain'):
                        p=active.ActivationDestroyProbe(f.runtime,f.client,f.resume.acceptance,observed,time.monotonic()+5)
                        self.assertEqual(p.prior['source'],candidate['source']);self.assertIn('launch_cpu_domain',p.prior['source'])
                        opened.assert_called_once_with(candidate['source']);p.pidfd=None;p.close()
                    else:
                        with self.assertRaises(ValueError):active.ActivationDestroyProbe(f.runtime,f.client,f.resume.acceptance,observed,time.monotonic()+5)
                        opened.assert_not_called()

    def test_full_terminal_successor_and_history_are_required_without_a_paused_precondition(self):
        for fault in ('none','version','context','retention','operation','old','local','float'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;f.client.current_acceptance=f.accepted_destroy
                p._bind_terminal_acceptance(f.accepted_destroy);inbox=copy.deepcopy(f.inbox);row=inbox['sessions'][0]
                if fault=='version':row['current_version']+=1
                if fault=='context':row['entries'][-1]['intent']['center']['center']='other'
                if fault=='retention':row['entries'][-1]['intent']['retention']['window']+=1
                if fault=='operation':row['entries'][-1]['intent']['request']['operation']={'kind':'renew'}
                if fault=='old':row['entries'][0]['applied_deadline']=17
                if fault=='local':row['local']['sandbox']+=1
                if fault=='float':row['entries'][-1]['intent']['accepted_version']=4.0
                with self.subTest(fault=fault):
                    if fault=='none':self.assertEqual(p._terminal_entries(inbox)[1]['intent']['request']['operation'],{'kind':'destroy'});self.assertFalse(p.candidate_paused)
                    else:
                        with self.assertRaises(ValueError):p._terminal_entries(inbox)

    def test_stop_barrier_requires_original_work_pin_positive_charge_and_current_fence(self):
        for fault in ('none','old-end','producer-release','pin','charge','source','capture','consumer-release','retiring','vm','epoch','lifetime','fence-file','fence-pending','extra-stop','prepare','activation','new-capture'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;s=copy.deepcopy(f.snapshot)
                if fault=='old-end':s['sources'][0]['capture']['end']={'state':'worker_ended','observation':{'controlled':'original end'}}
                if fault=='producer-release':s['sources'][0]['material_release']['producer']={'context':s['sources'][0]['restore']['local'],'input_sequence':1}
                if fault=='pin':s['sources'][0]['restore']['material_use']=False
                if fault=='charge':s['sources'][0]['capture']['reserved_material_bytes']-=1
                if fault=='source':s['sources'][0]['work']['anchor']['local']['operation']+=1
                if fault=='capture':s['sources'][0]['capture']['full']['memory_bytes']+=1
                if fault=='consumer-release':s['sources'][0]['material_release']['consumer']={'unknown':'release'}
                if fault=='retiring':s['sources'][0]['retirement']={'state':'retiring'}
                if fault=='vm':s['native']['current_vm']+=1
                if fault=='epoch':s['native']['epoch']+=1
                if fault=='lifetime':s['lifetime']['end_at_ms']+=1
                if fault=='fence-pending':s['terminal_actions'][0]=(s['terminal_actions'][0][0],'dispatched',None)
                if fault=='extra-stop':s['global_pending_stops'].append(copy.deepcopy(f.stop))
                if fault=='prepare':s['resume_actions'].append(copy.deepcopy(s['resume_actions'][0]))
                if fault=='activation':s['resume_actions'][1][0]['identity']['resource']['id']='other'
                if fault=='new-capture':s['terminal_actions'].append(({'command':{'logical':{'command':{'SealSnapshot':{}}}}},'ready',None))
                with patch.object(active,'original_json',return_value=dict(epoch=9 if fault=='fence-file' else 4,terminal=True)),self.subTest(fault=fault):
                    if fault in ('none','old-end','producer-release'):self.assertTrue(p._stop_barrier(s)['terminal'])
                    else:
                        with self.assertRaises(ValueError):p._stop_barrier(s)

    def test_cleanup_cut_accepts_same_late_activation_but_requires_exact_stop_and_kernel_exit(self):
        for fault in ('none','late-activation','foreign-activation','wrong-stop','no-exit','live','cleanup-saved','epoch','prepare-missing'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;p.recovered=p.stop_released=True;p.before_stop=f.snapshot
                p.terminal_host=Mock(pid=778);p.terminal_host.wait.return_value=86
                f.runtime.services.processes.children['host-0']=p.terminal_host
                f.runtime.services.processes.start=Mock(return_value=Mock(pid=779));f.runtime.services.wait_ready=Mock()
                s=copy.deepcopy(f.cut);p._read_terminal=Mock(return_value=s)
                if fault in ('late-activation','foreign-activation'):
                    a=s['resume_actions'][1][0];s['resume_actions'][1]=(a,'receipted',f.work_receipt(a,'Activated',dict(pid=124 if fault=='foreign-activation' else 123,start_ticks=456,boot_id='boot')))
                if fault=='wrong-stop':s['terminal_actions'][1][2]['outcome']['details']['result']['Stopped']['pid']+=1
                if fault=='cleanup-saved':s['terminal_actions'][2]=(s['terminal_actions'][2][0],'receipted',{})
                if fault=='epoch':s['terminal_actions'][2][0]['identity']['epoch']+=1
                if fault=='prepare-missing':s['resume_actions']=s['resume_actions'][1:]
                with patch.object(active,'pidfd_exited',return_value=fault!='no-exit'),patch.object(active,'original_process',return_value=fault=='live'),self.subTest(fault=fault):
                    if fault in ('none','late-activation'):self.assertTrue(p.assert_terminal_before_gc(p.deadline)['original_candidate_exited'])
                    else:
                        with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                        f.runtime.services.processes.start.assert_not_called()

    def test_real_sql_snapshot_correlates_full_inbox_global_stop_and_original_lifetime(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory));p=f.probe;f.client.current_acceptance=f.accepted_destroy;p._bind_terminal_acceptance(f.accepted_destroy)
            raw=json.dumps(f.inbox).encode();db=Path(f.runtime.services.native_pools[0]['database'])
            with sqlite3.connect(db) as conn:
                conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER); CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB,owner_id TEXT); CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB); CREATE TABLE sandbox_lifetimes(sandbox_id TEXT,owner_id TEXT,started_at_ms INTEGER,deadline_ms INTEGER,max_deadline_ms INTEGER,startup_deadline_ms INTEGER);')
                conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',(raw,hashlib.sha256(raw).digest(),65536))
                conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',(p.config['sandbox'],json.dumps(f.snapshot['native']).encode(),'tenant-1'))
                conn.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?)',(p.config['sandbox'],'tenant-1',*[p.prior['lifetime'][k] for k in ('started_at_ms','end_at_ms','max_end_at_ms','startup_deadline_ms')]))
                for i,(a,state,result) in enumerate(f.actions+f.snapshot['terminal_actions']):
                    conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?)',(i,p.config['sandbox'],a['identity']['operation'],json.dumps(a).encode(),state,None if result is None else json.dumps(result).encode()))
            db.chmod(0o600)
            with patch.object(capture,'checked_source_records',return_value=[f.record]):s=p._read_terminal(p.deadline)
            self.assertEqual(s['global_pending_stops'],[f.stop]);self.assertEqual(s['lifetime'],p.prior['lifetime'])
            with sqlite3.connect(db) as conn:conn.execute('UPDATE controller_lifecycle_inbox SET sha256=?',(b'x'*32,))
            with patch.object(capture,'checked_source_records',return_value=[f.record]),self.assertRaises(ValueError):p._read_terminal(p.deadline)

    def test_changed_complete_process_record_is_not_registered_or_restarted(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory));p=f.probe;f.client.current_acceptance=f.accepted_destroy
            f.runtime.services.processes.start=Mock();changed=copy.deepcopy(f.candidate);changed['source']['launch_cpu_domain']=[3]
            with patch.object(active,'pidfd_exited',return_value=False),patch.object(active,'original_process',return_value=True),patch.object(active,'observe_candidate',return_value=changed),self.assertRaises(ValueError):
                p.recover_after_destroy(f.accepted_destroy,p.deadline,observe_peer=Mock())
            f.runtime.services.processes.start.assert_not_called()
            self.assertEqual(f.runtime.runs[f.client.session]['source'],{'original':'old VMM'})


class ActualStopSocketTests(unittest.TestCase):
    def test_real_original_stop_peer_waits_for_pin_material_and_peer_progress(self):
        for fault in ('none','peer','extra-stop'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe;f.client.current_acceptance=f.accepted_destroy;events=[];received=[]
                server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);server.bind(str(p.stop_path));server.listen(1);server.settimeout(2)
                def serve():
                    connection,_=server.accept()
                    with connection:received.append(connection.recv(1))
                thread=threading.Thread(target=serve);thread.start()
                host=Mock(pid=os.getpid());host.poll.return_value=None
                def start(*_,**__):f.runtime.services.processes.children['host-0']=host;events.append('start');return host
                f.runtime.services.processes.start=Mock(side_effect=start);f.runtime.services.wait_ready=Mock()
                f.runtime.finish_resume_gate.side_effect=lambda *_:events.append('exit86')
                p._read_terminal=Mock(return_value=f.snapshot)
                if fault=='extra-stop':f.snapshot['global_pending_stops'].append(copy.deepcopy(f.stop))
                stack.enter_context(patch.object(active,'observe_candidate',return_value=f.candidate))
                stack.enter_context(patch.object(active,'original_process',return_value=True))
                stack.enter_context(patch.object(active,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(active,'original_json',return_value=dict(epoch=4,terminal=True)))
                def progress():
                    self.assertFalse(p.stop_released);self.assertEqual(received,[]);events.append('peer')
                    if fault=='peer':raise ValueError('peer failed')
                    return {'original':'guest progress'}
                try:
                    if fault=='none':
                        result=p.recover_after_destroy(f.accepted_destroy,p.deadline,observe_peer=progress)
                        self.assertTrue(result['original_pin_and_positive_charge_retained']);self.assertFalse(result['public_completion'])
                        self.assertLess(events.index('exit86'),events.index('start'));self.assertTrue(p.stop_released)
                        self.assertEqual(f.runtime.runs[f.client.session]['source'],f.source)
                    else:
                        with self.assertRaises(ValueError):p.recover_after_destroy(f.accepted_destroy,p.deadline,observe_peer=progress)
                        self.assertFalse(p.stop_released);self.assertFalse(p.terminal_failure_state()['capacity_released'])
                finally:p.pidfd=None;p.close();thread.join(3);server.close();self.assertFalse(thread.is_alive())
                self.assertEqual(received,[b'g' if fault=='none' else b''])


class ActivationExitTests(unittest.TestCase):
    def test_original_gate_exit_is_verified_without_starting_a_new_manager(self):
        for fault in ('none','code','dead','configuration'):
            u=runtime.UnloadRuntime.__new__(runtime.UnloadRuntime);host=Mock();host.wait.return_value=0 if fault=='code' else 86
            u.restore_gate={'host':host};u.release_resume_gate=Mock();u._gate_files_unchanged=Mock();u.configs={'host-0':{'original':True}}
            processes=NS(children={'host-0':host},bindings={'host-0':dict(config='/protected/host.json',uid=0,gid=0)},start=Mock())
            u.services=NS(processes=processes);receipt={'candidate':{'original':'identity'}}
            with patch.object(runtime,'original_process',return_value=fault!='dead'),patch.object(runtime,'role_facts',return_value={} if fault=='configuration' else u.configs['host-0']),self.subTest(fault=fault):
                if fault=='none':self.assertIs(u.finish_resume_gate(receipt,time.monotonic()+5),host);self.assertTrue(u.restore_gate['exit86_observed'])
                else:
                    with self.assertRaises(ValueError):u.finish_resume_gate(receipt,time.monotonic()+5)
                    self.assertNotIn('exit86_observed',u.restore_gate)
                processes.start.assert_not_called();u.release_resume_gate.assert_called_once_with('continue')


class ActualSceneTests(unittest.TestCase):
    def exercise(self,fault=False):
        events=[];clients={};lifetime=dict(started_at_ms=1,end_at_ms=2,max_end_at_ms=3,startup_deadline_ms=2)
        for owner in ('1','2'):
            c=Mock(spec=scene.PublicUnloadScenario);c.owner=owner;c.session='session-'+owner
            c.create_acceptance={};c.current_acceptance={};c.unload_acceptance={'key':{}};c.concurrent_unload_replies=[{},{}]
            c.resume_request={};c.witness={};c.timings=[dict(sent_monotonic=time.monotonic())];c.last_http_status=None;c.failure_step='controlled'
            c.timing_report.return_value=[];c.wait_running.return_value={'phase':'running'};c.wait_unloaded.return_value={'phase':'unloaded'}
            c.query.return_value={'phase':'resuming' if owner=='1' else 'running'}
            c.assert_terminal_resume_replay.side_effect=lambda *_,**kw:{'phase':kw['phase']}
            for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_continued_workload'):
                getattr(c,name).side_effect=lambda *_,n=name,o=owner,**__:events.append(n+'-'+o) or {}
            clients[owner]=c
        probe=Mock(spec=active.ActivationDestroyProbe)
        for name in ('assert_terminal_before_gc','cleanup','close'):
            getattr(probe,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        def recover(*_,observe_peer,**__):
            events.append('recover')
            if fault:raise ValueError('unknown original activated Stop')
            observe_peer();events.append('release-stop');return {}
        probe.recover_after_destroy.side_effect=recover;probe.terminal_failure_state.return_value={'original':'preserved'}
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='original',foreign_key='peer',services=NS(sdk_url='http://127.0.0.1:1'),
                measurement_identity=lambda:{},start=lambda _:None,create_parameters=lambda **_:{},
                record_running=Mock(side_effect=lambda session,*_,**__:dict(session=session,lifetime=lifetime,context={'position':0})),
                unload_evidence=Mock(return_value=dict(facts=lifetime,original_position={'position':0},original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation':{}}),confirm_running=Mock(return_value={}),
                arm_restore_gate=Mock(side_effect=lambda *_:events.append('arm')),wait_resume_gate=Mock(side_effect=lambda *_:events.append('actual-RAM') or {}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),finish=Mock(side_effect=lambda _:events.append('finish')),
                fail_resume_gate=Mock(return_value='preserved'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            stack.enter_context(patch.object(active,'ActivationDestroyProbe',return_value=probe))
            args=NS(scenario='restore-activation-destroy',scenario_timeout=60,activation_gate_timeout_ms=60000)
            if fault:
                with self.assertRaisesRegex(ValueError,'unknown original activated Stop'):scene.exercise(args)
                result=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(result['complete']);self.assertEqual(len(result['known_sessions']),2)
                self.assertIn('activation_destroy_failure_state',result);clients['2'].destroy.assert_not_called()
                runner.settlement_evidence.assert_not_called();probe.assert_terminal_before_gc.assert_not_called()
            else:
                result=scene.exercise(args);self.assertTrue(result['complete']);self.assertEqual(len(result['cycles']),1)
                self.assertTrue(result['cycles'][0]['destroyed_before_public_running'])
                self.assertLess(events.index('actual-RAM'),events.index('destroy-1'))
                self.assertLess(events.index('destroy-1'),events.index('recover'))
                self.assertLess(events.index('assert_continued_workload-2'),events.index('release-stop'))
                self.assertLess(events.index('assert_terminal_before_gc'),events.index('wait_cleaned-1'))
                self.assertLess(events.index('wait_cleaned-1'),events.index('destroy-2'))
                self.assertEqual([r['phase'] for r in result['terminal_resume_replays']],['terminating','cleaned'])
                self.assertNotIn('E7',result['acceptance_cases'])
            self.assertEqual(clients['1'].wait_running.call_count,1);clients['1'].assert_resumed_workload.assert_not_called()
            self.assertEqual(clients['1'].unload.call_count,1);self.assertEqual(clients['1'].resume.call_count,1)
            runner.finish.assert_called_once();probe.close.assert_called_once()

    def test_original_scene_reaches_activation_ram_then_destroy_stop_and_exact_gc(self):self.exercise()
    def test_unknown_stop_preserves_both_original_sessions_without_a_new_business_operation(self):self.exercise(fault=True)
