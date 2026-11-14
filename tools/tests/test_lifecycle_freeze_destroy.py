"""E8 consumers with original SQL/framing and controlled providers, never KVM."""
import copy
from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import signal
import sys
import tempfile
import threading
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock,patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_freeze_destroy as frozen
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
from tools.tests import test_internal_fault_relay as transport


def fixture(root):
    node,original,_=transport.InternalFaultRelayTests.pair(None)
    namespace='0123456789abcdef';source=dict(resource_id='vm-'+namespace+'-9',pid=123,start_time_ticks=456,boot_id='boot')
    local=dict(owner=1,sandbox=4,operation=3)
    context=dict(position=1,expected_previous=None,binding=dict(owner=1,sandbox=4,generation=2,vm=9,network=10))
    lifetime=dict(started_at_ms=900,end_at_ms=1000,max_end_at_ms=1100,startup_deadline_ms=1000)
    resume=copy.deepcopy(original);resume['accepted_version']=3
    resume['request']['binding']['key']['sequence']=3;resume['request']['binding']['expected']={'state':'version','version':2}
    resume['request']['operation']=dict(kind='resume',source_incarnation=1,capture=3,target_incarnation=7,manifest_digest=[7]*32)
    unload=copy.deepcopy(resume);unload['accepted_version']=4;unload['request']['binding']['key']['sequence']=4
    unload['request']['binding']['expected']['version']=3;unload['request']['operation']=dict(kind='unload',source_incarnation=7,capture=5)
    destroy=copy.deepcopy(unload);destroy['accepted_version']=5;destroy['request']['binding']['key']['sequence']=5
    destroy['request']['binding']['expected']['version']=4;destroy['request']['operation']={'kind':'destroy'}
    def accepted(i):return dict(session=i['request']['binding']['session'],key=copy.deepcopy(i['request']['binding']['key']),accepted_version=i['accepted_version'])
    def entry(i,op):return dict(intent=i,local_operation=op,applied_deadline=None,previous_effective_end_at_ms=None,unload_resolution=None)
    old=entry(unload,4);terminal=entry(destroy,5)
    def action(kind,epoch=3,operation=4):
        fields=dict(sandbox=4,generation=2,vm=9,epoch=epoch)
        if kind=='CleanupVm':fields.pop('epoch')
        if kind=='InstallFence':fields=dict(sandbox=4,epoch=epoch,terminal=True)
        return dict(identity=dict(protocol_version=1,owner='tenant-1',sandbox='sb-'+namespace+'-4',operation='op-'+namespace+'-4-'+str(operation),epoch=epoch,generation=2,attempt=0,
                    resource=dict(kind='sandbox' if kind=='InstallFence' else 'vm',id='sb-'+namespace+'-4' if kind=='InstallFence' else source['resource_id'])),
                    command=dict(attempt=0,logical=dict(command={kind:fields})))
    freeze=action('FreezeVm');fence=action('InstallFence',4,5);stop=action('StopVm',4,5);cleanup=action('CleanupVm',4,5)
    def receipt(a,kind,body):return dict(identity=copy.deepcopy(a['identity']),outcome=dict(status='observed_success',details=dict(resource=copy.deepcopy(a['identity']['resource']),command=copy.deepcopy(a['command']['logical']['command']),result={kind:body})))
    backing=dict(work=dict(anchor=dict(local=dict(local),request=original)),observation={'sample':1},
        capture=dict(full=dict(state_bytes=1,memory_bytes=2,disk_bytes=3),reserved_material_bytes=2*6+2*262144+65536,completed=[7]*32),
        restore=dict(intent=resume,material_use=True),retirement=None,material_release=dict(producer=None,consumer=None))
    work=dict(work=dict(anchor=dict(local=dict(local),request=unload),requirement=dict(resource=dict(kind='vm',id=source['resource_id']),node=node,source={k:source[k] for k in ('pid','start_time_ticks','boot_id')})),
        observation={'sample':1},capture=None,restore=None,retirement=None,material_release=dict(producer=None,consumer=None))
    rows=[backing,work]
    native=dict(terminal=True,phase='Revoking',generation=2,current_vm=9,net_slot=10,epoch=4,current_operation=dict(kind='Kill',id=5,epoch=4))
    snapshot=dict(original=old,terminal=terminal,local=local,native=native,sources=rows,original_actions=[(freeze,'dispatched',None)],
        terminal_actions=[(fence,'receipted',receipt(fence,'Fence',dict(epoch=4,terminal=True))),(stop,'dispatched',None)],
        global_pending_stops=[stop],lifetime=lifetime)
    cleanup_snapshot=copy.deepcopy(snapshot)
    cleanup_snapshot['terminal_actions'][1]=(stop,'receipted',receipt(stop,'Stopped',dict(pid=123,start_ticks=456)))
    cleanup_snapshot['terminal_actions'].append((cleanup,'dispatched',None));cleanup_snapshot['global_pending_stops']=[]
    settings=dict(namespace=namespace,database=str(root/'state.db'),node_state_dir=str(root/'state'),chroot_base=str(root/'jails'))
    config=dict(inbox=dict(node=node),native=dict(maintenance=dict(capacity=dict(max_sql_rows=64,max_sql_value_bytes=65536))))
    runner=NS(services=NS(native_pools=[settings],host_configs=[config],processes=NS(bindings={'host-0':dict(uid=os.getuid(),gid=os.getgid())})),
              quota=dict(deployment_control=str(root)),_materials=Mock(return_value={'original':'material'}),gc_complete=False)
    client=Mock(spec=scene.PublicUnloadScenario);client.session=accepted(unload)['session'];client.current_acceptance=accepted(destroy)
    client.destroy_request=dict(window=4,sequence=5,expected_version=4)
    p=frozen.FreezeDestroyProbe.__new__(frozen.FreezeDestroyProbe)
    p.runtime=runner;p.client=client;p.prior=dict(session=client.session,local=local,context=context,source=source,lifetime=lifetime,running_backing={'original':'material'})
    p.stage='freeze-receipt';p.running_acceptance=accepted(resume);p.accepted=accepted(unload);p.terminal_accepted=None
    p.unload_entry=copy.deepcopy(old);p.record=copy.deepcopy(work);p.action=copy.deepcopy(freeze)
    p.initial_sources=copy.deepcopy(rows);p.backing_material={'original':'material'}
    p.config=dict(sandbox='sb-'+namespace+'-4');p.deadline=time.monotonic()+5
    p.pidfd=999999;p.fd=None;p.stop_peer=None;p.stop_path=root/'stop.sock';p.stop_released=False;p.recovered=False;p.terminal_cut=None;p.pause_requested=False;p.candidate_paused=False;p.continue_sent=False
    p.host=Mock(pid=777);p.host.poll.return_value=86;p._configuration=Mock()
    inbox=dict(sessions=[dict(id=client.session,current_version=5,terminal=True,local=local,entries=[old,terminal])])
    return NS(probe=p,runner=runner,node=node,inbox=inbox,snapshot=snapshot,cleanup=cleanup_snapshot,accepted=accepted,
        unload=unload,destroy=destroy,freeze=freeze,stop=stop,receipt=receipt,rows=rows)


class FrozenFactsTests(unittest.TestCase):
    def test_original_freeze_cut_is_checked_before_holding_the_source(self):
        for fault in ('none','receipt','source','resource','pin','material'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe;p.accepted=None;p.host.wait.return_value=86
                if fault=='receipt':f.snapshot['original_actions'][0]=(f.freeze,'receipted',{})
                if fault=='source':f.rows[1]['work']['requirement']['source']['pid']+=1
                if fault=='resource':f.freeze['identity']['resource']['id']='vm-other'
                if fault=='pin':f.rows[0]['restore']['material_use']=False
                if fault=='material':f.runner._materials.return_value={'changed':'bytes'}
                p._read=Mock(return_value=(p.unload_entry,f.rows,f.snapshot['original_actions']))
                events=[];p._hold_source=Mock(side_effect=lambda _:events.append('hold'))
                pause=stack.enter_context(patch.object(frozen,'paused_source',side_effect=lambda *_:events.append('paused') or {'actual':'paused'}))
                stack.enter_context(patch.object(frozen,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(capture,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(capture,'original_process',return_value=True))
                with self.subTest(fault=fault):
                    if fault=='none':
                        result=p.after_acceptance(f.accepted(f.unload),p.deadline)
                        self.assertEqual(events,['paused','hold']);self.assertTrue(result['freeze_sql_receipt_pending'])
                        self.assertEqual(p.accepted,f.accepted(f.unload));self.assertEqual(p.action,f.freeze)
                        p.host.wait.assert_called_once()
                    else:
                        with self.assertRaises(ValueError):p.after_acceptance(f.accepted(f.unload),p.deadline)
                        pause.assert_not_called();p._hold_source.assert_not_called()

    def test_complete_destroy_history_context_and_window_are_correlated(self):
        with tempfile.TemporaryDirectory() as directory:
            for fault in ('none','request','owner','retention','center','expected','old-entry','version','local','float'):
                f=fixture(Path(directory));p=f.probe;p._bind_destroy(f.accepted(f.destroy));inbox=copy.deepcopy(f.inbox)
                row=inbox['sessions'][0];new=row['entries'][-1]['intent']
                if fault=='request':new['request']['operation']={'kind':'unload','source_incarnation':7,'capture':6}
                if fault=='owner':new['request']['binding']['key']['owner']='other'
                if fault=='retention':new['retention']['window']+=1
                if fault=='center':new['center']['center']='other'
                if fault=='expected':new['request']['binding']['expected']['version']-=1
                if fault=='old-entry':row['entries'][0]['unload_resolution']={'invented':'unloaded'}
                if fault=='version':row['current_version']+=1
                if fault=='local':row['local']['sandbox']+=1
                if fault=='float':new['accepted_version']=5.0
                with self.subTest(fault=fault):
                    if fault=='none':self.assertEqual(p._terminal_entries(inbox)[1]['intent'],f.destroy)
                    else:
                        with self.assertRaises(ValueError):p._terminal_entries(inbox)

    def test_one_current_stop_requires_real_fence_original_pin_charge_and_no_new_full(self):
        for fault in ('none','fence-pending','fence-file','stop-key','stop-other','stop-extra','vm','epoch','pin','charge','history','full','activate','lifetime','observation-refresh'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;s=copy.deepcopy(f.snapshot)
                if fault=='fence-pending':s['terminal_actions'][0]=(s['terminal_actions'][0][0],'dispatched',None)
                if fault=='stop-key':s['terminal_actions'][1][0]['identity']['resource']['id']='vm-other'
                if fault=='stop-other':s['global_pending_stops']=[]
                if fault=='stop-extra':s['global_pending_stops'].append(copy.deepcopy(f.stop))
                if fault=='vm':s['native']['current_vm']+=1
                if fault=='epoch':s['native']['epoch']+=1
                if fault=='pin':s['sources'][0]['restore']['material_use']=False
                if fault=='charge':s['sources'][0]['capture']['reserved_material_bytes']-=1
                if fault=='history':s['sources'][1]['work']['requirement']['source']['pid']+=1
                if fault=='full':s['sources'][1]['capture']={'action':f.freeze,'worker':None,'completed':None,'end':None}
                if fault=='activate':s['original_actions'].append(({'command':{'logical':{'command':{'ActivateVm':{}}}}},'ready',None))
                if fault=='lifetime':s['lifetime']['end_at_ms']+=1
                if fault=='observation-refresh':s['sources'][1]['observation']={'sample':2}
                with patch.object(frozen,'original_json',return_value=dict(epoch=9 if fault=='fence-file' else 4,terminal=True)),self.subTest(fault=fault):
                    if fault in ('none','observation-refresh'):self.assertTrue(p._stop_barrier(s)['terminal'])
                    else:
                        with self.assertRaises((ValueError,capture._AwaitTerminal)):p._stop_barrier(s)

    def test_cleanup_needs_original_stop_receipt_kernel_exit_and_pending_cleanup(self):
        for fault in ('none','pid','no-exit','still-live','cleanup-saved','cleanup-epoch','freeze-missing','extra-full'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;p.recovered=p.stop_released=p.continue_sent=True;p.before_stop=copy.deepcopy(f.snapshot)
                s=copy.deepcopy(f.cleanup);p.terminal_host=Mock();p.terminal_host.wait.return_value=86
                p._read_terminal=Mock(return_value=s);p.runtime.services.processes.start=Mock();p.runtime.services.wait_ready=Mock()
                if fault=='pid':s['terminal_actions'][1][2]['outcome']['details']['result']['Stopped']['pid']+=1
                if fault=='cleanup-saved':s['terminal_actions'][2]=(s['terminal_actions'][2][0],'receipted',{})
                if fault=='cleanup-epoch':s['terminal_actions'][2][0]['identity']['epoch']+=1
                if fault=='freeze-missing':s['original_actions']=[]
                if fault=='extra-full':s['sources'][1]['capture']={'action':f.freeze,'worker':None,'completed':None,'end':None}
                with patch.object(frozen,'pidfd_exited',return_value=fault!='no-exit'),patch.object(frozen,'original_process',return_value=fault=='still-live'),self.subTest(fault=fault):
                    if fault=='none':
                        value=p.assert_terminal_before_gc(p.deadline);self.assertTrue(value['original_source_exited'])
                        p.runtime.services.processes.start.assert_called_once_with('host-0')
                    else:
                        with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                        p.runtime.services.processes.start.assert_not_called()

    def test_real_sql_snapshot_keeps_exact_history_global_stop_and_lifetime(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory));p=f.probe;p._bind_destroy(f.accepted(f.destroy));raw=json.dumps(f.inbox).encode()
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER); CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB,owner_id TEXT); CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB); CREATE TABLE sandbox_lifetimes(sandbox_id TEXT,owner_id TEXT,started_at_ms INTEGER,deadline_ms INTEGER,max_deadline_ms INTEGER,startup_deadline_ms INTEGER);')
                conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',(raw,hashlib.sha256(raw).digest(),65536))
                name=p.config['sandbox'];conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',(name,json.dumps(dict(f.snapshot['native'],owner=1)).encode(),'tenant-1'))
                conn.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?)',(name,'tenant-1',900,1000,1100,1000))
                for i,(a,state,receipt) in enumerate(f.snapshot['original_actions']+f.snapshot['terminal_actions']):
                    conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?)',(i,name,a['identity']['operation'],json.dumps(a).encode(),state,None if receipt is None else json.dumps(receipt).encode()))
            Path(p.runtime.services.native_pools[0]['database']).chmod(0o600)
            with patch.object(capture,'checked_source_records',return_value=f.rows):
                got=p._read_terminal(p.deadline)
            self.assertEqual(got['original'],p.unload_entry);self.assertEqual(got['global_pending_stops'],[f.stop])
            self.assertEqual(got['lifetime'],p.prior['lifetime']);self.assertNotIn('resume_actions',got)
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:conn.execute('UPDATE controller_lifecycle_inbox SET sha256=?',(b'x'*32,))
            with patch.object(capture,'checked_source_records',return_value=f.rows),self.assertRaises(ValueError):p._read_terminal(p.deadline)


class OriginalSourceHoldTests(unittest.TestCase):
    def test_owned_child_is_held_by_original_pidfd_and_closing_never_continues_it(self):
        child=subprocess.Popen(['/bin/sleep','10'])
        descriptor=None
        try:
            source=dict(pid=child.pid,start_time_ticks=int((Path('/proc')/str(child.pid)/'stat').read_text().rsplit(')',1)[1].split()[19]),
                        boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            descriptor=runtime.open_original_candidate_pidfd(source)
            p=frozen.FreezeDestroyProbe.__new__(frozen.FreezeDestroyProbe)
            p.prior={'source':source};p.pidfd=descriptor;p.fd=None;p.stop_peer=None
            p.pause_requested=p.recovered=p.candidate_paused=p.continue_sent=False
            deadline=time.monotonic()+3
            p._hold_source(deadline)
            self.assertEqual(p._source_state(deadline),'T');self.assertTrue(p.pause_requested)
            self.assertTrue(p.candidate_paused);self.assertFalse(runtime.pidfd_exited(descriptor))
            with self.assertRaises(ValueError):p._hold_source(deadline)
            p.prior['source']['start_time_ticks']+=1
            p.pause_requested=False
            with patch.object(frozen.signal,'pidfd_send_signal') as send,self.assertRaises(ValueError):p._hold_source(deadline)
            send.assert_not_called();p.prior['source']['start_time_ticks']-=1
            p.close();descriptor=None
            self.assertEqual((Path('/proc')/str(child.pid)/'stat').read_text().rsplit(')',1)[1].split()[0],'T')
        finally:
            # This test owns the child. Product consumers never use this exit
            # cleanup as a VM or capacity observation.
            child.kill();child.wait(timeout=3)
            if descriptor is not None:os.close(descriptor)


class OriginalUnixPauseTests(unittest.TestCase):
    @contextmanager
    def endpoint(self,fault='none'):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);namespace='0123456789abcdef';resource='vm-'+namespace+'-9'
            jail=root/'firecracker'/resource/'root';jail.mkdir(parents=True)
            source=dict(resource_id=resource,pid=os.getpid(),start_time_ticks=int(Path('/proc/self/stat').read_text().rsplit(')',1)[1].split()[19]),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            if fault=='peer':source['pid']+=1
            claim=root/'.claims'/resource;claim.mkdir(parents=True);(claim/'record.json').write_text(json.dumps(source));(claim/'record.json').chmod(0o600)
            prior=dict(source=source,context=dict(position=0,binding={'vm':9}))
            runner=NS(services=NS(native_pools=[dict(namespace=namespace,chroot_base=str(root),firecracker='/fixed/firecracker',bounded_host={'uid_base':os.getuid()})]))
            server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
            descriptor=os.open(jail,os.O_RDONLY|os.O_DIRECTORY)
            try:server.bind('/proc/self/fd/'+str(descriptor)+'/api.socket')
            finally:os.close(descriptor)
            server.listen(1);server.settimeout(2)
            seen=[];errors=[]
            value=dict(app_name='Firecracker',id=resource,state='Paused',vmm_version='1.16.1')
            if fault=='running':value['state']='Running'
            if fault=='version':value['vmm_version']='other'
            if fault=='resource':value['id']='vm-other'
            if fault=='extra':value['guessed_exit']=True
            def serve():
                try:
                    connection,_=server.accept()
                    with connection:
                        connection.settimeout(2);request=connection.recv(4096)
                        if not request:return
                        seen.append(request)
                        data=json.dumps(value).encode();headers=b'HTTP/1.1 200 OK\r\nContent-Length: '+str(len(data)).encode()+b'\r\n'
                        if fault=='length':headers+=b'Content-Length: 1\r\n'
                        connection.sendall(headers+b'\r\n'+data)
                except BaseException as error:errors.append(type(error).__name__)
            thread=threading.Thread(target=serve);thread.start()
            try:yield runner,prior,seen
            finally:thread.join(3);server.close();self.assertFalse(thread.is_alive());self.assertEqual(errors,[])

    def test_actual_unix_peer_and_single_get_bind_original_paused_source(self):
        with self.endpoint() as (runner,prior,seen):
            result=frozen.paused_source(runner,prior,time.monotonic()+2)
            self.assertEqual(result['peer_pid'],os.getpid());self.assertEqual(result['state'],'Paused')
            self.assertFalse(result['execution_authority']);self.assertEqual(len(seen),1);self.assertTrue(seen[0].startswith(b'GET / HTTP/1.1'))

    def test_running_wrong_peer_identity_version_and_framing_never_establish_frozen(self):
        for fault in ('running','peer','version','resource','extra','length'):
            with self.subTest(fault=fault),self.endpoint(fault) as (runner,prior,seen),patch.object(frozen,'original_process',return_value=True):
                with self.assertRaises(ValueError):frozen.paused_source(runner,prior,time.monotonic()+2)


class FreezeStopWiringTests(unittest.TestCase):
    def test_actual_stop_socket_is_released_only_after_checks_and_peer_progress(self):
        for fault in ('none','peer-progress','global-stop'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe;events=[];received=[]
                server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);server.bind(str(p.stop_path));server.listen(1);server.settimeout(2)
                def serve():
                    connection,_=server.accept()
                    with connection:received.append(connection.recv(1))
                thread=threading.Thread(target=serve);thread.start()
                host=Mock(pid=os.getpid());host.poll.return_value=None
                p.runtime.services.processes.start=Mock(side_effect=lambda *_,**__:events.append('start') or host)
                p.runtime.services.wait_ready=Mock();p._read_terminal=Mock(return_value=f.snapshot)
                if fault=='global-stop':f.snapshot['global_pending_stops']=[]
                stack.enter_context(patch.object(frozen,'paused_source',side_effect=lambda *_:events.append('paused') or {'original':'paused'}))
                p._source_state=Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal',side_effect=lambda *_:events.append('continue')))
                stack.enter_context(patch.object(frozen,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(frozen,'original_json',return_value=dict(epoch=4,terminal=True)))
                def progress():
                    self.assertFalse(p.stop_released);self.assertEqual(received,[]);events.append('peer')
                    if fault=='peer-progress':raise ValueError('unknown peer')
                    return {'original':'progress'}
                try:
                    if fault=='none':
                        result=p.recover_after_destroy(f.accepted(f.destroy),p.deadline,observe_peer=progress)
                        self.assertTrue(result['original_pin_and_positive_charge_retained']);self.assertTrue(p.stop_released)
                        self.assertLess(events.index('peer'),events.index('continue'));self.assertLess(events.index('continue'),events.index('paused'))
                    else:
                        with self.assertRaises(ValueError):p.recover_after_destroy(f.accepted(f.destroy),p.deadline,observe_peer=progress)
                        self.assertFalse(p.stop_released);self.assertFalse(p.recovered)
                        self.assertFalse(p.terminal_failure_state()['capacity_released'])
                finally:
                    p.pidfd=None;p.close();thread.join(3);server.close();self.assertFalse(thread.is_alive())
                self.assertEqual(received,[b'g' if fault=='none' else b''])


class SceneAndReplayTests(unittest.TestCase):
    def test_terminal_unload_replay_retains_the_actual_original_replay_checks(self):
        for phase in ('terminating','cleaned'):
            c=scene.PublicUnloadScenario.__new__(scene.PublicUnloadScenario);c.session='original'
            c.current_acceptance=dict(session=c.session,key={'sequence':5},accepted_version=5)
            c.unload_acceptance=dict(session=c.session,key={'sequence':4},accepted_version=4)
            current=dict(session=c.session,operation=c.current_acceptance['key'],current_version=5,
                         phase=phase,unload_outcome=None,create_refusal=None)
            c.query=Mock(return_value=current);c.assert_original_replay=Mock()
            c.assert_data_closed=Mock();c.assert_previous_tokens_closed=Mock()
            result=c.assert_terminal_unload_replay(phase=phase)
            c.assert_original_replay.assert_called_once_with(current)
            self.assertTrue(result['terminal_view_unchanged'])
            for field,value in (('phase','running'),('current_version',4),('unload_outcome',{'state':'unloaded'})):
                changed=dict(current);changed[field]=value;c.query.return_value=changed
                with self.subTest(phase=phase,field=field),self.assertRaises(AssertionError):c.assert_terminal_unload_replay(phase=phase)

    def exercise(self,fault=False):
        events=[];clients={};lifetime=dict(started_at_ms=1,end_at_ms=2,max_end_at_ms=3,startup_deadline_ms=2)
        for owner in ('1','2'):
            c=Mock(spec=scene.PublicUnloadScenario);c.owner=owner;c.session='session-'+owner
            c.create_acceptance={};c.current_acceptance={};c.unload_acceptance={'key':{}}
            c.concurrent_unload_replies=[{},{}];c.resume_request={};c.witness={};c.timings=[dict(sent_monotonic=time.monotonic())]
            c.last_http_status=None;c.failure_step='controlled';c.timing_report.return_value=[]
            c.wait_running.return_value={'phase':'running'};c.wait_unloaded.return_value={'phase':'unloaded'}
            c.query.return_value={'phase':'unloading' if owner=='1' else 'running'}
            c.assert_terminal_unload_replay.side_effect=lambda *_,**kw: {'phase':kw['phase']}
            for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_resumed_workload','assert_continued_workload'):
                getattr(c,name).side_effect=lambda *_,n=name,o=owner,**__:events.append(n+'-'+o) or {}
            clients[owner]=c
        calls=0
        def running(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context={'position':0 if session=='session-2' or calls==1 else 1})
        p=Mock(spec=frozen.FreezeDestroyProbe)
        for name in ('arm','after_acceptance','assert_terminal_before_gc','cleanup','close'):
            getattr(p,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        def recovery(*_,observe_peer,**__):
            events.append('recover')
            if fault:raise ValueError('unknown original Stop')
            observe_peer();events.append('release-stop');return {}
        p.recover_after_destroy.side_effect=recovery
        p.terminal_failure_state.return_value={'original':'preserved'}
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='original',foreign_key='peer',services=NS(sdk_url='http://127.0.0.1:1'),
                measurement_identity=lambda:{},start=lambda _:None,create_parameters=lambda **_:{},
                record_running=Mock(side_effect=running),unload_evidence=Mock(return_value=dict(facts=lifetime,original_position={'position':0},original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation':{}}),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),
                finish=Mock(side_effect=lambda _:events.append('finish')),fail_resume_gate=Mock(return_value='unarmed'),
                close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            stack.enter_context(patch.object(frozen,'FreezeDestroyProbe',return_value=p))
            args=NS(scenario='capture-freeze-destroy',scenario_timeout=60,activation_gate_timeout_ms=60000)
            if fault:
                with self.assertRaisesRegex(ValueError,'unknown original Stop'):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete']);self.assertEqual(len(report['known_sessions']),2)
                self.assertEqual(clients['1'].destroy.call_count,1);clients['2'].destroy.assert_not_called()
                p.assert_terminal_before_gc.assert_not_called();runner.settlement_evidence.assert_not_called()
                self.assertEqual(clients['1'].unload.call_count,2);self.assertEqual(clients['1'].resume.call_count,1)
                self.assertIn('freeze_destroy_failure_state',report)
            else:
                report=scene.exercise(args);self.assertTrue(report['complete']);self.assertEqual(len(report['cycles']),1)
                self.assertTrue(report['cycles'][0]['frozen_unload_destroyed'])
                self.assertLess(events.index('after_acceptance'),events.index('destroy-1'))
                self.assertLess(events.index('destroy-1'),events.index('recover'))
                self.assertLess(events.index('assert_continued_workload-2'),events.index('release-stop'))
                self.assertLess(events.index('release-stop'),events.index('assert_terminal_before_gc'))
                self.assertLess(events.index('assert_terminal_before_gc'),events.index('wait_cleaned-1'))
                self.assertLess(events.index('wait_cleaned-1'),events.index('destroy-2'))
                self.assertEqual(clients['1'].resume.call_count,1);self.assertIn('E8',report['acceptance_cases'])
            runner.finish.assert_called_once();p.close.assert_called_once()
            self.assertLess(events.index('finish'),events.index('close'))

    def test_real_scene_keeps_peer_until_original_terminal_stop_exit_and_cleanup(self):self.exercise()
    def test_unknown_stop_keeps_original_failure_without_reissuing_or_destroying_peer(self):self.exercise(fault=True)
