"""Decoded audit/transport/driver tests; controlled facts are not KVM acceptance."""
import copy
import io
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
from contextlib import ExitStack
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock,patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import _internal_fault_relay as relay
import lifecycle_resume_receipt_loss as resume
import lifecycle_unload_scenario as scene
import lifecycle_unload_runtime as runtime
from tools.tests import test_lifecycle_unload_receipt_loss as previous
from tools.tests import test_internal_fault_relay as transport


def fixture():
    old,audit,processes,unloaded=previous.audit_fixture()
    baseline=copy.deepcopy(audit);row=baseline['sandboxes'][0]
    row['phase']='unloaded';row['operations'][-1]['unloaded']=copy.deepcopy(unloaded)
    request=copy.deepcopy(unloaded['request']);request['accepted_version']=3
    request['request']['binding']['key']['sequence']=3
    request['request']['binding']['expected']['version']=2
    request['request']['operation']=dict(kind='resume',source_incarnation=1,capture=3,
        target_incarnation=7,manifest_digest=[7]*32)
    facts=dict(old.lifetime,observed_at_ms=971)
    result=dict(request=copy.deepcopy(request),current_request=copy.deepcopy(request),readiness=dict(state='observed',facts=facts))
    pending=copy.deepcopy(baseline);pending['operation_highwater']+=1
    current=copy.deepcopy(row['operations'][-1]);current.update(
        binding=request['request']['binding'],operation=request['request']['operation'],center=request['center'],
        retention=request['retention'],accepted_version=3,acceptance_time={'kind':'sampled','at_ms':970},unloaded=None)
    pending['sandboxes'][0]['operations'].append(copy.deepcopy(current));pending['sandboxes'][0]['phase']='resuming'
    accepted=dict(session=request['request']['binding']['session'],key=request['request']['binding']['key'],accepted_version=3)
    observer=resume.ResumeRestartObserver(node=old.node,acceptance=accepted,create_identity=old.create_identity,
        lifetime=old.lifetime,baseline=baseline)
    recovered=copy.deepcopy(pending);row=recovered['sandboxes'][0];row['phase']='running'
    row['last_running']=dict(facts,observed_at_ms=980);row['operations'][-1]['running']=copy.deepcopy(row['last_running'])
    return NS(node=old.node,request=request,result=result,baseline=baseline,pending=pending,recovered=recovered,
              processes=processes,observer=observer,acceptance=accepted,lifetime=old.lifetime,unloaded=unloaded)


class ResumeGateTests(unittest.TestCase):
    def test_only_exact_current_observed_resume_opens_dormant_one_shot_gate(self):
        f=fixture();gate=relay.ResumeReceiptBarrier(f.node)
        self.assertIsNone(gate.observe(f.request,f.result));gate.arm()
        for state in ('pending','state_lost','not_current','expired','not_running'):
            value=dict(f.result,readiness={'state':state})
            self.assertIsNone(gate.observe(f.request,value));self.assertFalse(gate.ready.is_set())
        token=gate.observe(f.request,f.result);self.assertTrue(gate.ready.is_set())
        with self.assertRaises(ValueError):gate.loss_evidence()
        gate.unblock(discard=True)
        with self.assertRaises(ValueError):gate.loss_evidence()
        gate.finish(token,dropped=True,failed=False)
        self.assertEqual(gate.loss_evidence()['observation'],f.result)
        self.assertEqual(gate.loss_evidence()['observed_at_ms'],[971])
        with self.assertRaises(RuntimeError):gate.arm()

    def test_every_original_request_field_is_correlated_without_key_or_session_shortcuts(self):
        f=fixture()
        paths=[('binding','protocol_version'),('binding','node','deployment_id'),('binding','node','node_id'),
               ('binding','node','node_generation'),('binding','session'),('binding','key','deployment'),
               ('binding','key','owner'),('binding','key','window'),('binding','key','sequence'),
               ('binding','expected','version'),('operation','source_incarnation'),('operation','capture'),
               ('operation','target_incarnation'),('operation','manifest_digest')]
        for path in paths:
            bad=copy.deepcopy(f.result);value=bad['request']['request']
            for name in path[:-1]:value=value[name]
            old=value[path[-1]];value[path[-1]]=[8]*32 if isinstance(old,list) else old+1 if type(old) is int else 'different'
            gate=relay.ResumeReceiptBarrier(f.node);gate.arm()
            with self.subTest(path=path),self.assertRaises(ValueError):gate.observe(f.request,bad)
            self.assertFalse(gate.ready.is_set())
        for key in ('accepted_version','center','retention'):
            bad=copy.deepcopy(f.result)
            if key=='accepted_version':bad['request'][key]+=1
            else:bad['request'][key][next(iter(bad['request'][key]))]='different'
            with self.assertRaises(ValueError):relay.resumed_observation(f.request,bad,f.node)
        self.assertTrue(relay.resumed_observation(f.request,f.result,f.node))  # incarnation hole + distinct windows

    def test_current_scope_facts_missing_extra_and_untyped_fields_cannot_trigger(self):
        f=fixture()
        for fault in ('owner','center','retention','node','nan-time','bool-time','end','extra','missing'):
            bad=copy.deepcopy(f.result)
            if fault=='owner':bad['current_request']['request']['binding']['key']['owner']='elsewhere'
            if fault=='center':bad['current_request']['center']['center']='elsewhere'
            if fault=='retention':bad['current_request']['retention']['window']+=1
            if fault=='node':bad['current_request']['request']['binding']['node']['node_id']='other'
            if fault=='nan-time':bad['readiness']['facts']['observed_at_ms']=float('nan')
            if fault=='bool-time':bad['readiness']['facts']['observed_at_ms']=True
            if fault=='end':bad['readiness']['facts']['observed_at_ms']=f.lifetime['end_at_ms']
            if fault=='extra':bad['readiness']['facts']['candidate_released']=True
            if fault=='missing':del bad['readiness']['facts']['started_at_ms']
            gate=relay.ResumeReceiptBarrier(f.node);gate.arm()
            with self.subTest(fault=fault),self.assertRaises(ValueError):gate.observe(f.request,bad)
            self.assertFalse(gate.ready.is_set())

    def test_only_valid_observation_time_varies_and_eight_total_receipts_are_bounded(self):
        f=fixture();gate=relay.ResumeReceiptBarrier(f.node);gate.arm();tokens=[]
        for timestamp in (971,973,972):
            value=copy.deepcopy(f.result);value['readiness']['facts']['observed_at_ms']=timestamp
            tokens.append(gate.observe(f.request,value))
        gate.unblock(discard=True)
        for token in tokens:gate.finish(token,dropped=True,failed=False)
        evidence=gate.loss_evidence();self.assertEqual(evidence['observation'],f.result)
        self.assertEqual(evidence['observed_at_ms'],[971,973,972])
        gate=relay.ResumeReceiptBarrier(f.node);gate.arm()
        for _ in range(8):
            token=gate.observe(f.request,f.result);gate.finish(token,dropped=False,failed=True)
        with self.assertRaises(ValueError):gate.observe(f.request,f.result)
        self.assertEqual(len(gate.observed_at_ms),8)
        for key in f.lifetime:
            gate=relay.ResumeReceiptBarrier(f.node);gate.arm();gate.observe(f.request,f.result)
            value=copy.deepcopy(f.result);value['readiness']['facts'][key]+=1
            with self.subTest(key=key),self.assertRaises(ValueError):gate.observe(f.request,value)

    def test_actual_handler_discards_before_reply_and_closes_backend_before_drained(self):
        f=fixture();h,backend=transport.InternalFaultRelayTests().handler(relay.RESUME_OBSERVATION,f.request,f.result)
        h.server.relay.resume_barrier=relay.ResumeReceiptBarrier(f.node);gate=h.server.relay.resume_barrier;gate.arm()
        def close():self.assertFalse(gate.drained.is_set())
        backend.close.side_effect=close
        with patch.object(relay,'_Backend',return_value=backend),patch.object(gate.release,'wait',side_effect=lambda **_:gate.unblock(discard=True) or True):
            h._forward()
        h._reply.assert_not_called();self.assertTrue(gate.drained.is_set())
        self.assertEqual(gate.loss_evidence()['dropped'],1)
        self.assertEqual(json.loads(backend.request.call_args.kwargs['body']),f.request)

    def test_timeout_forwarding_or_cleanup_failure_never_establish_loss(self):
        f=fixture()
        for mode in ('timeout','forward','close-failed'):
            h,backend=transport.InternalFaultRelayTests().handler(relay.RESUME_OBSERVATION,f.request,f.result)
            h.server.relay.resume_barrier=relay.ResumeReceiptBarrier(f.node);gate=h.server.relay.resume_barrier;gate.arm()
            if mode=='close-failed':backend.close.side_effect=OSError('owned close failed')
            wait=lambda **_:False if mode=='timeout' else (gate.unblock(discard=mode=='close-failed') or True)
            with patch.object(relay,'_Backend',return_value=backend),patch.object(gate.release,'wait',side_effect=wait):
                if mode=='close-failed':
                    with self.assertRaises(OSError):h._forward()
                else:h._forward()
            gate.unblock(discard=True)
            with self.assertRaises(ValueError):gate.loss_evidence()


class ResumeObserverTests(unittest.TestCase):
    def stopped(self):
        f=fixture();f.observer.hold(f.result,f.processes)
        f.observer.center_stopped(f.pending,{k:v for k,v in f.processes.items() if k!='center'})
        return f

    def restarted(self):
        f=self.stopped();f.observer.discarded(dict(observation=f.result,held=2,dropped=2,response_attempts=0,observed_at_ms=[971,972]))
        after=copy.deepcopy(f.processes);after['center'].update(pid=104,start_time_ticks=101);f.observer.restarted(after);return f

    def test_same_source_and_history_survive_with_a_fresh_real_readonly_time(self):
        f=self.restarted();value=f.observer.recovered(f.recovered)
        self.assertEqual(value['recovered_running']['observed_at_ms'],980)
        self.assertEqual(value['acceptance'],f.acceptance)
        self.assertEqual(value['observation'],f.result)
        self.assertEqual(f.observer.phase,'recovered')

    def test_pending_requires_resuming_no_saved_running_no_refund_and_same_prior_history(self):
        for fault in ('running','source','create','reservation','already-saved','extra-op','retention'):
            f=fixture();f.observer.hold(f.result,f.processes);bad=copy.deepcopy(f.pending);row=bad['sandboxes'][0]
            if fault=='running':row['phase']='running'
            if fault=='source':row['operations'][1]['unloaded']['outcome']['facts']['manifest_digest']=[9]*32
            if fault=='create':row['create']['options_sha256']='changed'
            if fault=='reservation':bad['reserved_resources']['instances']=0
            if fault=='already-saved':row['operations'][-1]['running']=f.result['readiness']['facts']
            if fault=='extra-op':row['operations'].append(copy.deepcopy(row['operations'][-1]))
            if fault=='retention':row['operations'][-1]['retention']['window']+=1
            with self.subTest(fault=fault),self.assertRaises(ValueError):f.observer.center_stopped(bad,{k:v for k,v in f.processes.items() if k!='center'})

    def test_recovery_rejects_changed_result_history_peer_and_logical_resources(self):
        for fault in ('lifetime','history','manifest','reservation','context','peer'):
            f=self.restarted();bad=copy.deepcopy(f.recovered);row=bad['sandboxes'][0]
            if fault=='lifetime':row['operations'][-1]['running']['end_at_ms']-=1
            if fault=='history':row['operations'][0]['acceptance_time']['at_ms']+=1
            if fault=='manifest':row['operations'][-1]['operation']['manifest_digest']=[8]*32
            if fault=='reservation':bad['reserved_resources']['cpu']+=1
            if fault=='context':row['operations'][-1]['center']['center']='other'
            if fault=='peer':bad['sandboxes'].append(copy.deepcopy(row));bad['sandboxes'][-1]['create']['binding']['session']='other'
            with self.subTest(fault=fault),self.assertRaises(ValueError):f.observer.recovered(bad)

    def test_source_facts_are_independent_and_timestamp_counts_are_not_fabricated(self):
        f=fixture();bad=copy.deepcopy(f.result);bad['readiness']['facts']['end_at_ms']-=1
        with self.assertRaises(ValueError):f.observer.hold(bad,f.processes)
        for samples in ([971], [True,972], [971,1000], [], [971]*9):
            f=self.stopped()
            with self.subTest(samples=samples),self.assertRaises(ValueError):
                f.observer.discarded(dict(observation=f.result,held=2,dropped=2,response_attempts=0,observed_at_ms=samples))


class ResumeWiringTests(unittest.TestCase):
    def test_original_pipeline_binds_candidate_before_stop_and_preserves_unknown_replacement(self):
        for fault in ('none','hold-failed','replacement'):
            f=fixture();events=[];probe=resume.ResumeReceiptRestart.__new__(resume.ResumeReceiptRestart)
            probe.node=f.node;probe.baseline=f.baseline;probe.create_identity=f.observer.create_identity
            probe.closed=False;probe.started=True;probe.deadline=time.monotonic()+5
            probe.observer=None;probe.evidence=dict(complete=False,preparation={})
            gate=relay.ResumeReceiptBarrier(f.node);gate.arm();token=gate.observe(f.request,f.result)
            after=copy.deepcopy(f.processes);after['center'].update(pid=104,start_time_ticks=101)
            processes=NS(observe=Mock(side_effect=[f.processes,after,after,after]),
                stop=Mock(side_effect=lambda name,**_:events.append('stop-'+name)),
                start=Mock(side_effect=lambda name:events.append('start-'+name)),audit=Mock(return_value=f.pending))
            probe.services=NS(processes=processes,wait_ready=Mock(),
                planned_audit=Mock(side_effect=lambda *_,**__:events.append('post-audit') or f.recovered))
            probe.runtime=NS();probe._configs=Mock()
            probe.relay=NS(resume_barrier=gate,failed=threading.Event(),resume_loss_evidence=gate.loss_evidence)
            unblock=gate.unblock
            def drop(*,discard):
                events.append('drop');unblock(discard=discard);gate.finish(token,dropped=True,failed=False)
            candidate=dict(source={'original':'process'},context={'original':'position'},claim={'original':'pin'})
            def sample(*_):
                events.append('candidate')
                if fault=='hold-failed':raise ValueError('unknown original candidate')
                return dict(candidate,source={'replacement':True}) if fault=='replacement' and events.count('candidate')==2 else candidate
            with patch.object(gate,'unblock',side_effect=drop),patch.object(resume,'observe_candidate',side_effect=sample), \
                    patch('lifecycle_unload_receipt_loss.observe_survivors',return_value={k:v for k,v in f.processes.items() if k!='center'}):
                if fault!='none':
                    with self.assertRaises(ValueError):probe.interrupt(f.acceptance,dict(lifetime=f.lifetime),probe.deadline)
                    self.assertFalse(probe.evidence['complete'])
                    if fault=='hold-failed':processes.stop.assert_not_called();processes.start.assert_not_called()
                else:
                    probe.interrupt(f.acceptance,dict(lifetime=f.lifetime),probe.deadline)
                    public=dict(session=f.acceptance['session'],operation=f.acceptance['key'],current_version=3,
                                phase='running',unload_outcome=None,create_refusal=None)
                    probe.recovered(public,probe.deadline)
                    self.assertTrue(probe.evidence['complete'])
                    self.assertEqual(events,['candidate','stop-center','drop','start-center','candidate','post-audit'])
                    processes.stop.assert_called_once_with('center',force=True)
                    processes.start.assert_called_once_with('center')
            if token in gate.active:unblock(discard=True);gate.finish(token,dropped=True,failed=False)


class ResumeCandidateTests(unittest.TestCase):
    def test_original_readonly_sql_mapping_pin_and_activation_bind_the_same_candidate(self):
        f=fixture()
        for fault in ('none','pin','pin-type','activation','intent','owner','context','resource','operation','attempt','dead'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                root=Path(directory);database=root/'state.db'
                context=dict(position=1,expected_previous=None,binding=dict(owner=1,sandbox=4,generation=2,vm=9,network=10))
                action=dict(identity=dict(protocol_version=1,attempt=0,operation='op-0123456789abcdef-4-3',owner='tenant-1',sandbox='sb-0123456789abcdef-4',generation=2,epoch=3,
                    resource={'kind':'vm','id':'vm-0123456789abcdef-9'}),
                    command={'attempt':0,'logical':{'command':{'ActivateVm':dict(sandbox=4,generation=2,vm=9,epoch=3)}}})
                claim=dict(intent=copy.deepcopy(f.request),local={'owner':1,'sandbox':4,'operation':3},generation=2,material_use=True,activation=action)
                if fault=='pin':claim['material_use']=False
                if fault=='pin-type':claim['material_use']=1
                if fault=='operation':action['identity']['operation']='op-other'
                if fault=='attempt':action['identity']['attempt']=1
                if fault=='activation':claim['activation']=None
                if fault=='intent':claim['intent']['retention']['window']+=1
                if fault=='owner':action['identity']['owner']='tenant-2'
                if fault=='context':context['binding']['vm']=10
                if fault=='resource':action['identity']['resource']['id']='vm-other'
                with sqlite3.connect(database) as conn:
                    conn.execute('CREATE TABLE controller_host_slots(position INTEGER,context_json BLOB)')
                    conn.execute('INSERT INTO controller_host_slots VALUES(?,?)',(1,json.dumps(context).encode()))
                database.chmod(0o600)
                native=dict(database=str(database),namespace='0123456789abcdef',chroot_base=str(root),bounded_host={'maintenance':{'max_host_slots':2}})
                config=dict(native={'maintenance':{'capacity':{'max_sql_value_bytes':8192}}})
                runner=NS(services=NS(native_pools=[native],host_configs=[config]))
                reader=stack.enter_context(patch.object(resume,'checked_source_records',return_value=[dict(restore=claim,capture={'completed':[7]*32})]))
                stack.enter_context(patch.object(resume,'original_json',return_value=dict(resource_id='vm-0123456789abcdef-9',pid=123,start_time_ticks=456,boot_id='owned')))
                stack.enter_context(patch.object(resume,'original_process',return_value=fault!='dead'))
                import server_bounded_churn
                live=stack.enter_context(patch.object(server_bounded_churn,'live_identity',return_value={'original':'physical'}))
                if fault=='none':
                    value=resume.observe_candidate(runner,f.request,time.monotonic()+5)
                    self.assertEqual(value['claim'],claim);live.assert_called_once();reader.assert_called_once()
                else:
                    with self.subTest(fault=fault),self.assertRaises(ValueError):resume.observe_candidate(runner,f.request,time.monotonic()+5)


class ResumeSceneTests(unittest.TestCase):
    def _exercise(self,fail=False):
        import lifecycle_unload_cow as cow
        import lifecycle_unload_retirement as retirement
        f=fixture();events=[];clients={}
        for owner in ('1','2'):
            c=Mock(spec=scene.PublicUnloadScenario);c.owner=owner;c.session='session-'+owner
            c.create_acceptance={};c.current_acceptance={};c.unload_acceptance={'key':{}}
            c.concurrent_unload_replies=[{},{}];c.resume_request={};c.witness={};c.timings=[dict(sent_monotonic=time.monotonic())]
            c.timing_report.return_value=[];c.query.return_value={'phase':'running'};c.wait_running.return_value={'phase':'running'}
            c.wait_unloaded.return_value={'phase':'unloaded'};c.last_http_status=None;c.failure_step='controlled'
            for name in ('create','unload','resume','destroy','run_public_workload','assert_resumed_workload'):
                getattr(c,name).side_effect=lambda *_,o=owner,n=name,**__:events.append(n+'-'+o) or {}
            clients[owner]=c
        calls=0
        def physical(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=f.lifetime,context=dict(position=0 if session=='session-2' or calls==1 else 1))
        probe=Mock(spec=resume.ResumeReceiptRestart)
        for name in ('arm','interrupt','recovered','assert_candidate','close'):
            getattr(probe,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        if fail:
            def unknown(*_,**__):
                events.append('interrupt');raise ValueError('unknown original candidate')
            probe.interrupt.side_effect=unknown
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='owner',foreign_key='foreign',receipt_restart=probe,
                services=NS(sdk_url='http://127.0.0.1:1'),measurement_identity=lambda:{},start=lambda _:None,create_parameters=lambda **_:{},
                record_running=Mock(side_effect=physical),unload_evidence=Mock(return_value=dict(facts=f.lifetime,original_position={'position':0},original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation':{}}),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),finish=Mock(side_effect=lambda _:events.append('finish')),
                fail_resume_gate=Mock(return_value='unknown-preserved'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            sealed=Mock();sealed.verify_after_guest_write.return_value={}
            stack.enter_context(patch.object(cow,'SealedSourceProbe',return_value=sealed))
            stack.enter_context(patch.object(retirement,'wait_settled',return_value={'receipt':{},'snapshot':{}}))
            stack.enter_context(patch.object(retirement,'assert_replay_unchanged'))
            args=NS(scenario='resume-receipt-restart',scenario_timeout=60,activation_gate_timeout_ms=60000)
            if fail:
                with self.assertRaisesRegex(ValueError,'unknown original candidate'):scene.exercise(args)
                result=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(result['complete']);self.assertEqual(result['failure_type'],'ValueError')
                self.assertEqual(len(result['known_sessions']),2);self.assertEqual(result['cycles'],[])
                self.assertEqual(events.count('resume-1'),1)
                clients['1'].destroy.assert_not_called();clients['2'].destroy.assert_not_called()
                probe.recovered.assert_not_called();probe.assert_candidate.assert_not_called()
                probe.close.assert_called_once();runner.finish.assert_called_once()
                self.assertLess(events.index('finish'),events.index('close'));return
            result=scene.exercise(args)
            self.assertTrue(result['complete']);self.assertEqual(len(result['cycles']),3)
            for name in ('arm','interrupt','recovered','assert_candidate','close'):getattr(probe,name).assert_called_once()
            self.assertLess(events.index('arm'),events.index('resume-1'))
            self.assertLess(events.index('resume-1'),events.index('interrupt'))
            self.assertLess(events.index('interrupt'),events.index('recovered'))
            self.assertLess(events.index('recovered'),events.index('assert_candidate'))
            self.assertLess(events.index('finish'),events.index('close'))
            self.assertEqual(events.count('destroy-1'),1);self.assertEqual(events.count('destroy-2'),1)
            self.assertIn('Resume',result['acceptance_cases']['E13'])

    def test_original_three_cycles_reach_resume_fault_recovery_and_keep_cleanup_order(self):
        self._exercise()

    def test_unknown_candidate_preserves_both_sessions_without_reissue_or_forced_destroy(self):
        self._exercise(fail=True)
