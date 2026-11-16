"""Original audit DTO and inherited frozen consumers; no VM or clock changes."""
import copy
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_freeze_expiry as expiry
import lifecycle_freeze_destroy as frozen
from tools.tests import test_lifecycle_freeze_destroy as original
from tools.tests import test_internal_fault_relay as transport


def fixture(root):
    f = original.fixture(root)
    _, create, _ = transport.InternalFaultRelayTests.pair(None)
    first_unload = copy.deepcopy(create)
    first_unload['accepted_version'] = 2
    first_unload['request']['binding'].update(key=dict(create['request']['binding']['key'], sequence=2),
                                            expected={'state':'version', 'version':1})
    first_unload['request']['operation'] = dict(kind='unload', source_incarnation=1, capture=3)
    f.rows[0]['work']['anchor']['request'] = first_unload
    resume = f.rows[0]['restore']['intent']
    intents = [first_unload, resume, f.unload]
    lifetime = copy.deepcopy(f.probe.prior['lifetime'])

    def operation(intent):
        op = copy.deepcopy(intent['request']['operation'])
        if op['kind'] == 'create':
            op.pop('create')  # Exact Rust OperationKind redaction, not full Create serde.
        return dict(center=copy.deepcopy(intent['center']), binding=copy.deepcopy(intent['request']['binding']),
            retention=copy.deepcopy(intent['retention']), accepted_version=intent['accepted_version'],
            operation=op, acceptance_time={'kind':'sampled', 'at_ms':950}, running=None,
            create_refusal=None, terminal=None, unloaded=None)

    operations = [operation(i) for i in [create]+intents]
    operations[0]['running'] = dict(lifetime, observed_at_ms=950)
    operations[1]['unloaded'] = {'request':copy.deepcopy(first_unload), 'outcome':{'state':'unloaded'}}
    operations[2]['running'] = dict(lifetime, observed_at_ms=970)
    shape = create['request']['operation']['create']
    creation = dict(binding=copy.deepcopy(create['request']['binding']),
        **{k:copy.deepcopy(v) for k,v in shape.items() if k!='options'},
        options_sha256=hashlib.sha256(bytes(shape['options'])).hexdigest(),
        metadata_sha256=hashlib.sha256(b'{}').hexdigest())
    row = dict(create=creation, private=False, operations=operations, terminating=False,
               released=False, phase='unloading', last_running=copy.deepcopy(operations[2]['running']),
               terminal_confirmation=None)
    before = dict(sandboxes=[row])
    after = copy.deepcopy(before)
    terminal = operation(f.destroy)
    terminal.update(acceptance_time={'kind':'unsampled'})
    terminal['binding']['key'].update(window=2, sequence=77)
    after['sandboxes'][0].update(operations=copy.deepcopy(operations)+[terminal],
                               terminating=True, phase='terminating')
    p = expiry.FreezeExpiryProbe.__new__(expiry.FreezeExpiryProbe)
    p.__dict__.update(f.probe.__dict__)
    p.initial_sources = copy.deepcopy(f.rows)
    p.client.create_acceptance = f.accepted(create)
    p.client.current_acceptance = f.accepted(f.unload)
    p.client.destroy_request = None
    p.expiry_baseline = expiry.frozen_baseline(before,p.client.create_acceptance,intents,f.node,lifetime)
    p.automatic_acceptance = None
    p._source_state = Mock(return_value='T')
    center = Mock();center.poll.return_value = None
    processes = Mock();processes.children = {'center':center}
    processes.bindings = p.runtime.services.processes.bindings
    processes.audit.return_value = after
    p.runtime.services.processes = processes
    terminal_intent=dict(request=dict(binding=copy.deepcopy(terminal['binding']),operation={'kind':'destroy'}),
        accepted_version=5,center=copy.deepcopy(terminal['center']),retention=copy.deepcopy(terminal['retention']))
    f.inbox['sessions'][0]['entries'][-1]['intent']=copy.deepcopy(terminal_intent)
    for snapshot in (f.snapshot,f.cleanup):
        snapshot['terminal']['intent']=copy.deepcopy(terminal_intent)
        snapshot['sources']=copy.deepcopy(f.rows)
    result=vars(f).copy()
    result.update(probe=p,before=before,after=after,intents=intents,node=f.node,lifetime=lifetime,
        create=p.client.create_acceptance,processes=processes,center=center,
        accepted=dict(session=p.client.session,key=copy.deepcopy(terminal['binding']['key']),accepted_version=5))
    return NS(**result)


class FrozenExpiryAuditTests(unittest.TestCase):
    def test_original_full_intents_and_internal_key_keep_independent_retention_window(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            self.assertEqual(f.probe.expiry_baseline, f.before['sandboxes'][0])
            self.assertEqual(expiry.expiry_acceptance(f.after,f.probe.expiry_baseline,f.node,f.lifetime,1000),f.accepted)
            self.assertEqual(f.accepted['key']['window'],2)
            self.assertEqual(f.after['sandboxes'][0]['operations'][-1]['retention']['window'],7)
            self.assertIsNone(expiry.expiry_acceptance(f.before,f.probe.expiry_baseline,f.node,f.lifetime,1000))

    def test_all_three_original_operation_parameters_bind_to_independent_host_intents(self):
        changes = [(0,'source_incarnation',2),(0,'capture',4),(1,'source_incarnation',2),
                   (1,'capture',4),(1,'target_incarnation',8),(1,'manifest_digest',[8]*32),
                   (2,'source_incarnation',8),(2,'capture',6)]
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory))
            for index,field,value in changes:
                audit=copy.deepcopy(f.before)
                audit['sandboxes'][0]['operations'][index+1]['operation'][field]=value
                with self.subTest(index=index,field=field),self.assertRaises(ValueError):
                    expiry.frozen_baseline(audit,f.create,f.intents,f.node,f.lifetime)

    def test_jointly_changed_center_context_cannot_replace_original_host_context(self):
        for fault in ('node','node-generation','protocol','session','center','slot','window','expected','version','key'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));audit=copy.deepcopy(f.before)
                for op in audit['sandboxes'][0]['operations'][1:]:
                    if fault=='node':op['binding']['node']['node_id']='other'
                    if fault=='node-generation':op['binding']['node']['node_generation']+=1
                    if fault=='protocol':op['binding']['protocol_version']=3
                    if fault=='session':op['binding']['session']='other'
                    if fault=='center':op['center']['center']='other'
                    if fault=='slot':op['retention']['slot']+=1
                    if fault=='window':op['retention']['window']+=1
                    if fault=='expected':op['binding']['expected']['version']+=1
                    if fault=='version':op['accepted_version']+=1
                    if fault=='key':op['binding']['key']['sequence']+=1
                with self.subTest(fault=fault),self.assertRaises(ValueError):
                    expiry.frozen_baseline(audit,f.create,f.intents,f.node,f.lifetime)

    def test_pending_and_terminal_audits_preserve_entire_target_history_and_lifetime(self):
        for terminal in (False,True):
            for fault in ('history','running','lifetime','create-timeout','create-hash','private','released','phase','confirmation','duplicate','missing'):
                with tempfile.TemporaryDirectory() as directory:
                    f=fixture(Path(directory));audit=copy.deepcopy(f.after if terminal else f.before);row=audit['sandboxes'][0]
                    if fault=='history':row['operations'][1]['unloaded']['outcome']['state']='changed'
                    if fault=='running':row['operations'][2]['running']['observed_at_ms']+=1
                    if fault=='lifetime':row['last_running']['end_at_ms']+=1
                    if fault=='create-timeout':row['create']['timeout_ms']+=1
                    if fault=='create-hash':row['create']['options_sha256']='changed'
                    if fault=='private':row['private']=True
                    if fault=='released':row['released']=True
                    if fault=='phase':row['phase']='running'
                    if fault=='confirmation':row['terminal_confirmation']={}
                    if fault=='duplicate':audit['sandboxes'].append(copy.deepcopy(row))
                    if fault=='missing':audit['sandboxes']=[]
                    with self.subTest(terminal=terminal,fault=fault),self.assertRaises(ValueError):
                        expiry.expiry_acceptance(audit,f.probe.expiry_baseline,f.node,f.lifetime,1000)

    def test_every_terminal_field_and_json_type_is_checked(self):
        for fault in ('owner','deployment','key-window','key-sequence','node','node-generation','protocol','session','center','slot',
                      'retention-window','expected','version','extra-parameter','sampled','running','unloaded','refusal','receipt','float','bool'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));audit=copy.deepcopy(f.after);op=audit['sandboxes'][0]['operations'][-1]
                if fault=='owner':op['binding']['key']['owner']='other'
                if fault=='deployment':op['binding']['key']['deployment']='other'
                if fault=='key-window':op['binding']['key']['window']=4
                if fault=='key-sequence':op['binding']['key']['sequence']=0
                if fault=='node':op['binding']['node']['node_id']='other'
                if fault=='node-generation':op['binding']['node']['node_generation']=1.0
                if fault=='protocol':op['binding']['protocol_version']=3
                if fault=='session':op['binding']['session']='other'
                if fault=='center':op['center']['center']='other'
                if fault=='slot':op['retention']['slot']+=1
                if fault=='retention-window':op['retention']['window']+=1
                if fault=='expected':op['binding']['expected']['version']=3
                if fault=='version':op['accepted_version']=6
                if fault=='extra-parameter':op['operation']['capture']=5
                if fault=='sampled':op['acceptance_time']={'kind':'sampled','at_ms':1000}
                if fault=='running':op['running']={}
                if fault=='unloaded':op['unloaded']={}
                if fault=='refusal':op['create_refusal']='changed'
                if fault=='receipt':op['terminal']={}
                if fault=='float':op['accepted_version']=5.0
                if fault=='bool':op['binding']['key']['window']=True
                with self.subTest(fault=fault),self.assertRaises(ValueError):
                    expiry.expiry_acceptance(audit,f.probe.expiry_baseline,f.node,f.lifetime,1000)

    def test_no_automatic_acceptance_before_real_persisted_deadline(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory))
            for now in (999,True,1000.0):
                with self.subTest(now=now),self.assertRaises(ValueError):
                    expiry.expiry_acceptance(f.after,f.probe.expiry_baseline,f.node,f.lifetime,now)


class FrozenExpiryObservationTests(unittest.TestCase):
    def test_baseline_audit_finishes_before_expiry_and_restarts_original_center(self):
        for times in ([999,999],[1000],[999,1000]):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe
                f.processes.audit.return_value=f.before
                stack.enter_context(patch.object(frozen.FreezeDestroyProbe,'after_acceptance',return_value={'original':'Freeze'}))
                stack.enter_context(patch.object(expiry,'now_ms',side_effect=times))
                if len(times)==2 and times[-1]==999:
                    self.assertTrue(p.after_acceptance(p.accepted,p.deadline)['boundary_before_real_expiry'])
                else:
                    with self.assertRaises(ValueError):p.after_acceptance(p.accepted,p.deadline)
                self.assertEqual(f.processes.stop.call_count,0 if times==[1000] else 1)
                self.assertEqual(f.processes.start.call_count,0 if times==[1000] else 1)

    def test_live_center_crosses_ttl_before_audit_and_only_observed_key_is_bound(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));p=f.probe;events=[]
            def clock():
                events.append('clock');return 999 if len(events)==1 else 1000
            f.processes.stop.side_effect=lambda *_:events.append('stop-center')
            f.processes.audit.side_effect=lambda *_:events.append('audit') or f.after
            f.processes.start.side_effect=lambda *_:events.append('start-center')
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=clock))
            result=p.observe_expiry(p.deadline)
            self.assertEqual(events[:5],['clock','clock','stop-center','audit','start-center'])
            self.assertEqual(result['acceptance'],f.accepted)
            self.assertEqual(p.client.current_acceptance,f.accepted)
            self.assertFalse(result['explicit_destroy_sent'])
            self.assertIs(type(result['observed_monotonic']),float)
            self.assertLessEqual(result['observed_monotonic'],time.monotonic())
            p.client.query.assert_not_called();p.client.destroy.assert_not_called()
            p._bind_destroy(f.accepted);self.assertEqual(p.terminal_accepted,f.accepted)
            self.assertEqual(p._terminal_entries(f.inbox)[1]['intent']['request']['binding']['key'],f.accepted['key'])
            with self.assertRaises(ValueError):p._bind_destroy(f.accepted)

    def test_pending_original_audit_is_polled_but_never_promoted_to_acceptance(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));p=f.probe
            f.processes.audit.side_effect=[f.before,f.after]
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,1000,1000,1000,1000]))
            sleep=stack.enter_context(patch.object(expiry.time,'sleep'))
            result=p.observe_expiry(p.deadline)
            self.assertEqual(result['center_audits'],2);sleep.assert_called_once()
            self.assertEqual(f.processes.start.call_count,2)

    def test_explicit_destroy_early_expiry_or_dead_center_cannot_certify_autonomous_expiry(self):
        for fault in ('destroy','early','center','host','source','changed-current'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe
                if fault=='destroy':p.client.destroy_request={}
                if fault=='center':f.center.poll.return_value=1
                if fault=='host':p.host.poll.return_value=0
                if fault=='source':p._source_state.return_value='R'
                if fault=='changed-current':p.client.current_acceptance={}
                stack.enter_context(patch.object(expiry,'now_ms',side_effect=[1000] if fault=='early' else [999,1000]))
                with self.subTest(fault=fault),self.assertRaises(ValueError):p.observe_expiry(p.deadline)
                f.processes.audit.assert_not_called();p.client.query.assert_not_called();p.client.destroy.assert_not_called()
                self.assertIsNone(p.automatic_acceptance);self.assertFalse(p.stop_released)

    def test_audit_failure_restarts_center_and_keeps_original_unload_and_hold(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));p=f.probe;prior=copy.deepcopy(p.client.current_acceptance)
            f.processes.audit.side_effect=ValueError('invalid original audit')
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,1000]))
            with self.assertRaises(ValueError):p.observe_expiry(p.deadline)
            f.processes.start.assert_called_once_with('center')
            self.assertEqual(p.client.current_acceptance,prior)
            self.assertIsNone(p.automatic_acceptance);self.assertFalse(p.stop_released);self.assertFalse(p.continue_sent)

    def test_unobserved_or_changed_terminal_cannot_enter_inherited_stop_consumer(self):
        for fault in ('unobserved','destroy','key','current'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe
                if fault!='unobserved':p.automatic_acceptance=copy.deepcopy(f.accepted)
                p.client.current_acceptance=copy.deepcopy(f.accepted)
                if fault=='destroy':p.client.destroy_request={}
                value=copy.deepcopy(f.accepted)
                if fault=='key':value['key']['sequence']+=1
                if fault=='current':p.client.current_acceptance['accepted_version']+=1
                with self.subTest(fault=fault),self.assertRaises(ValueError):p._bind_destroy(value)
                self.assertIsNone(p.terminal_accepted)

    def test_automatic_completion_only_queries_and_never_fakes_destroy_timing(self):
        client=Mock();client.path.return_value='/original';client.owner_key='owner'
        client.call.side_effect=[(503,b''),(200,b'{"phase":"terminating"}'),(200,b'{"phase":"cleaned"}')]
        with patch.object(expiry.time,'sleep'):
            self.assertEqual(expiry.wait_expired_cleaned(client,time.monotonic()+1),{'phase':'cleaned'})
        self.assertTrue(all(c.args[0]=='GET' for c in client.call.call_args_list))
        client.destroy.assert_not_called();client.operation_completed.assert_not_called();client.operation_evidence_completed.assert_not_called()
        for reply in ((404,b''),(200,b'{"phase":"running"}'),(200,b'{"phase":"cleaned","phase":"cleaned"}')):
            client.call.side_effect=None;client.call.return_value=reply
            with self.subTest(reply=reply),self.assertRaises(ValueError):
                expiry.wait_expired_cleaned(client,time.monotonic()+1)


class FrozenExpirySceneTests(unittest.TestCase):
    def test_scene_uses_real_peer_consumer_and_no_first_owner_destroy(self):
        original.SceneAndReplayTests().exercise(expiry=True)

    def test_unknown_stop_preserves_two_sessions_without_business_retry(self):
        original.SceneAndReplayTests().exercise(expiry=True,fault=True)

    def test_stalled_peer_does_not_release_original_stop(self):
        original.SceneAndReplayTests().exercise(expiry=True,peer_fault='held-counter')

    def test_changed_peer_authorization_after_expiry_does_not_destroy_peer(self):
        original.SceneAndReplayTests().exercise(expiry=True,peer_fault='after-cleaned-token')
