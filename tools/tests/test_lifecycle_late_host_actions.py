"""Finite scene wiring, real SQLite capture and fail-closed observer composition."""
import copy
from contextlib import ExitStack, closing
import hashlib
from itertools import product
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
    context = dict(position=0, expected_previous=1, binding=dict(owner=1,sandbox=1,generation=2,vm=2,network=9))
    prior = dict(session='session-A',context=context,local=dict(owner=1,sandbox=1),
        source=dict(pid=41,start_time_ticks=11,resource_id='vm-original-2'))
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
    resource = dict(kind='vm',id='vm-original-2')
    def action(kind):
        command = dict(sandbox=1,generation=2,vm=2)
        if kind == 'StopVm': command['epoch']=5
        return dict(identity=dict(owner='tenant-1',sandbox='sb-original-1',operation='op-original-1-4',
            protocol_version=1,generation=2,epoch=5,resource=resource,attempt=1,
            logical_action='effect-1' if kind == 'StopVm' else 'effect-2'),
            command=dict(attempt=1,logical=dict(command={kind:command})))
    stop, cleanup = action('StopVm'),action('CleanupVm')
    result = dict(identity=copy.deepcopy(stop['identity']),outcome=dict(status='observed_success',
        details=dict(resource=resource,command=copy.deepcopy(stop['command']['logical']['command']),
            result=dict(Stopped=dict(pid=41,start_ticks=11)))))
    rows = [(stop,'receipted',result),(cleanup,'dispatched',None)]
    return prior,resume,original,accepted,inbox,rows,resource


def slot_rows(rows):
    configured = copy.deepcopy(rows[0][0])
    configured['identity'].update(operation='op-original-1-3', epoch=4, logical_action='effect-3',
                                  resource=dict(kind='network_slot', id='slot-original-9'))
    configured['command']['logical']['command'] = {'ConfigureNetwork':
        dict(sandbox=1, generation=2, vm=2, slot=9, epoch=4)}
    success = dict(identity=copy.deepcopy(configured['identity']), outcome=dict(status='observed_success',
        details=dict(resource=copy.deepcopy(configured['identity']['resource']),
                     command=copy.deepcopy(configured['command']['logical']['command']),
                     result={'Network': {'namespace_inode': 25}})))
    close = copy.deepcopy(rows[1][0])
    close['identity'].update(logical_action='effect-4', resource=copy.deepcopy(configured['identity']['resource']))
    close['command']['logical']['command'] = {'CleanupSlot': dict(sandbox=1, generation=2, slot=9)}
    return (configured, 'receipted', success), (close, 'dispatched', None)


class OriginalLateActionsTests(unittest.TestCase):
    def test_cleanup_slot_arms_from_the_successful_original_network_configuration(self):
        prior, resume, original, _, _, rows, _ = fixture()
        for fault in (None, 'missing', 'wrong-slot', 'float-inode', 'unknown'):
            with self.subTest(fault=fault):
                configured, _ = slot_rows(rows)
                processes = Mock()
                runner = NS(services=NS(native_pools=[dict(namespace='original')], host_configs=[{}],
                                        processes=processes, wait_ready=Mock()))
                client = NS(session=prior['session'], current_acceptance=resume)
                probe = late.LateSlotCloseProbe(runner, client, prior, resume, {}, time.monotonic()+5)
                if fault == 'wrong-slot':
                    configured[0]['command']['logical']['command']['ConfigureNetwork']['slot'] = 10
                if fault == 'float-inode':
                    configured[2]['outcome']['details']['result']['Network']['namespace_inode'] = 25.0
                if fault == 'unknown':
                    configured[2]['outcome']['status'] = 'unknown'
                probe._read = Mock(return_value=(original, {}, [] if fault == 'missing' else [configured]))
                probe._configuration = Mock()
                if fault is None:
                    self.assertEqual(probe.arm()['cut'], 'cleanup_slot-success-before-SQL-receipt')
                    self.assertEqual(probe.resource, dict(kind='network_slot', id='slot-original-9'))
                    processes.stop.assert_called_once_with('host-0')
                    processes.start.assert_called_once_with('host-0',
                        env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'cleanup_slot'})
                else:
                    with self.assertRaises(ValueError):
                        probe.arm()
                    processes.stop.assert_not_called()
                    processes.start.assert_not_called()

    def test_cleanup_slot_capture_requires_the_original_dispatched_slot_and_exit86(self):
        prior, resume, original, accepted, inbox, rows, _ = fixture()
        for fault in (None, 'missing', 'duplicate', 'wrong-slot', 'float-slot', 'wrong-resource',
                      'wrong-epoch', 'present-receipt', 'ready', 'old-live', 'wrong-exit'):
            with self.subTest(fault=fault):
                configured, pending = slot_rows(rows)
                client = NS(session=prior['session'], current_acceptance=resume,
                            destroy_request=dict(window=1, sequence=4, expected_version=3))
                runner = NS(services=NS(native_pools=[dict(namespace='original')], host_configs=[{}], processes=Mock()))
                probe = late.LateSlotCloseProbe(runner, client, prior, resume, {}, time.monotonic()+5)
                probe.entry = original
                probe.resource = probe._original_resource([configured])
                probe.host = Mock()
                probe.host.poll.return_value = 87 if fault == 'wrong-exit' else 86
                runner.services.processes.children = {'host-0': probe.host}
                action, state, result = pending
                if fault in ('wrong-slot', 'float-slot'):
                    action['command']['logical']['command']['CleanupSlot']['slot'] = 10 if fault == 'wrong-slot' else 9.0
                if fault == 'wrong-resource':
                    action['identity']['resource']['id'] = 'slot-original-10'
                if fault == 'wrong-epoch':
                    action['identity']['epoch'] = 5.0
                if fault == 'present-receipt':
                    result = {}
                if fault == 'ready':
                    state = 'ready'
                saved = [(action, state, result)]
                if fault == 'duplicate':
                    saved.append(copy.deepcopy(saved[0]))
                if fault == 'missing':
                    saved.clear()
                entry = inbox['sessions'][0]['entries'][-1]
                probe._read = Mock(return_value=(entry, {'epoch': 5}, [*rows, *saved]))
                client.current_acceptance = accepted
                with patch.object(late, 'original_process', return_value=fault == 'old-live'):
                    if fault is None:
                        evidence = probe.capture(accepted)
                        self.assertEqual(evidence['actual_commands'], ['CleanupSlot'])
                        self.assertIs(evidence['original_slot_close_executed_receipt_pending'], True)
                        self.assertEqual(probe.actions, [action])
                        self.assertIsNot(probe.actions[0], action)
                    else:
                        with self.assertRaises(ValueError):
                            probe.capture(accepted)
                        self.assertIsNone(probe.actions)

    def test_native_protocol_and_source_vm_are_independent_of_lifecycle_version(self):
        prior,_,original,_,_,rows,resource=fixture()
        self.assertEqual(original['intent']['request']['binding']['protocol_version'],4)
        self.assertEqual(rows[0][0]['identity']['protocol_version'],1)
        self.assertEqual(late.original_actions(rows,prior,{'epoch':5},resource),
                         [rows[0][0],rows[1][0]])
        for version in (4,True,1.0):
            bad=copy.deepcopy(rows)
            for action,_,result in bad:
                action['identity']['protocol_version']=version
                if result is not None:result['identity']['protocol_version']=version
            with self.subTest(version=version),self.assertRaises(ValueError):
                late.original_actions(bad,prior,{'epoch':5},resource)
        for changed in (dict(kind='sandbox',id=resource['id']),
                        dict(kind='vm',id='vm-original-3')):
            bad=copy.deepcopy(rows)
            for action,_,result in bad:
                action['identity']['resource']=copy.deepcopy(changed)
                if result is not None:
                    result['identity']['resource']=copy.deepcopy(changed)
                    result['outcome']['details']['resource']=copy.deepcopy(changed)
            with self.subTest(resource=changed),self.assertRaises(ValueError):
                late.original_actions(bad,prior,{'epoch':5},changed)

    def test_capture_uses_complete_actual_actions_and_receipts(self):
        prior,_,_,_,_,rows,resource = fixture()
        selected = late.original_actions(rows,prior,{'epoch':5},resource)
        self.assertEqual(selected,[rows[0][0],rows[1][0]])
        selected[0]['identity']['attempt']=8
        self.assertEqual(rows[0][0]['identity']['attempt'],1)
        for fault in ('missing','duplicate','vm','generation','resource','epoch','command',
                      'vm-float','sandbox-bool','epoch-float','receipt-ticks-float',
                      'receipt-pid','receipt-identity','receipt-command','unknown','cleanup-receipted'):
            bad=copy.deepcopy(rows)
            if fault=='missing': bad.pop()
            elif fault=='duplicate': bad.append(copy.deepcopy(bad[0]))
            elif fault in ('vm','generation'): bad[1][0]['command']['logical']['command']['CleanupVm'][fault]+=1
            elif fault in ('resource','epoch'): bad[1][0]['identity'][fault]='foreign'
            elif fault=='command': bad[0][0]['command']['logical']['command']['StopVm']['epoch']=8
            elif fault=='vm-float': bad[1][0]['command']['logical']['command']['CleanupVm']['vm']=2.0
            elif fault=='sandbox-bool': bad[1][0]['command']['logical']['command']['CleanupVm']['sandbox']=True
            elif fault=='epoch-float': bad[1][0]['identity']['epoch']=5.0
            elif fault=='receipt-ticks-float': bad[0][2]['outcome']['details']['result']['Stopped']['start_ticks']=11.0
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
        for fault in ('current-version','terminal','local','key','session','node','operation-missing',
                      'current-version-float','entry-version-float','local-owner-bool','key-sequence-float',
                      'operation-float'):
            bad=copy.deepcopy(inbox); current=bad['sessions'][0]
            if fault=='current-version': current['current_version']+=1
            elif fault=='terminal': current['terminal']=False
            elif fault=='local': current['local']['sandbox']=2
            elif fault=='key': current['entries'][-1]['intent']['request']['binding']['key']['sequence']+=1
            elif fault in ('session','node'): current['entries'][-1]['intent']['request']['binding'][fault]='foreign'
            elif fault=='current-version-float': current['current_version']=4.0
            elif fault=='entry-version-float': current['entries'][-1]['intent']['accepted_version']=4.0
            elif fault=='local-owner-bool': current['local']['owner']=True
            elif fault=='key-sequence-float': current['entries'][-1]['intent']['request']['binding']['key']['sequence']=4.0
            elif fault=='operation-float': current['entries'][-1]['local_operation']=4.0
            else: current['entries'][-1]['local_operation']=None
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                late.accepted_entry(bad,accepted,prior,node,terminal=True)
        client=NS(session=prior['session'],current_acceptance=resume,
            destroy_request=dict(window=1,sequence=4,expected_version=3))
        runner=NS(services=NS(native_pools=[{}],host_configs=[{}],processes=Mock()))
        for fault in ('none','center','retention','expected','protocol','retention-float','operation','local-operation',
                      'host-exit','host-replaced','old-live'):
            probe=late.LateHostActionsProbe(runner,client,prior,resume,{},time.monotonic()+5)
            probe.entry=original;probe.resource=resource;probe.host=Mock()
            probe.host.poll.return_value=86
            runner.services.processes.children={'host-0':probe.host}
            bad=copy.deepcopy(entry); badrows=copy.deepcopy(rows)
            if fault in ('center','retention'): bad['intent'][fault]={'foreign':True}
            elif fault=='retention-float': bad['intent']['retention']['window']=1.0
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
                    self.assertIsNone(probe.actions)
            client.current_acceptance=resume

    def test_prepare_identity_types_are_checked_before_restarting_the_host(self):
        prior, resume, original, _, _, rows, _ = fixture()
        for vm in (2, 2.0):
            with self.subTest(vm=vm):
                processes = Mock()
                runner = NS(services=NS(native_pools=[{}], host_configs=[{}], processes=processes,
                                        wait_ready=Mock()))
                client = NS(session=prior['session'], current_acceptance=resume)
                probe = late.LateHostActionsProbe(runner, client, prior, resume, {}, time.monotonic()+5)
                prepare = copy.deepcopy(rows[0][0])
                prepare['command']['logical']['command'] = {'PrepareVm': dict(sandbox=1, generation=2, vm=vm)}
                probe._read = Mock(return_value=(original, {}, [(prepare, 'receipted', None)]))
                probe._configuration = Mock()
                if type(vm) is int:
                    self.assertTrue(probe.arm()['preparatory_host_restart'])
                    processes.stop.assert_called_once_with('host-0')
                    processes.start.assert_called_once()
                else:
                    with self.assertRaisesRegex(ValueError, 'original persisted Prepare'):
                        probe.arm()
                    processes.stop.assert_not_called()
                    processes.start.assert_not_called()

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
            with closing(sqlite3.connect(database)) as conn, conn:
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
                for stored in ('null', b'null'):
                    with self.subTest(receipt=stored):
                        with closing(sqlite3.connect(database)) as conn, conn:
                            conn.execute('UPDATE outbox SET receipt_json=? WHERE id=1',(stored,))
                        with self.assertRaisesRegex(ValueError, 'stored outbox receipt cannot be JSON null'):
                            probe._read(accepted,terminal=True)
                with closing(sqlite3.connect(database)) as conn, conn:
                    conn.execute('UPDATE outbox SET receipt_json=NULL WHERE id=1')
                self.assertEqual(probe._read(accepted,terminal=True)[2],rows)
                state=dict(owner=1,generation=2,epoch=5,terminal=True,current_operation=dict(kind='Kill',id=4,epoch=5))
                for fault in ('checksum','bound','attempt','owner','operation','protocol_version','generation',
                              'native-kill','native-owner-bool','native-epoch-float','native-kill-id-float',
                              'attempt-float'):
                    with closing(sqlite3.connect(database)) as conn, conn:
                        conn.execute('UPDATE controller_lifecycle_inbox SET sha256=?,reserve_bytes=16384',
                            (hashlib.sha256(raw).digest(),))
                        conn.execute('UPDATE outbox SET action_json=? WHERE id=0',(json.dumps(rows[0][0]),))
                        conn.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(state),))
                        if fault=='checksum':conn.execute('UPDATE controller_lifecycle_inbox SET sha256=zeroblob(32)')
                        elif fault=='bound':conn.execute('UPDATE controller_lifecycle_inbox SET reserve_bytes=1')
                        elif fault.startswith('native-'):
                            bad=copy.deepcopy(state)
                            if fault=='native-kill': bad['current_operation']['id']=5
                            elif fault=='native-owner-bool': bad['owner']=True
                            elif fault=='native-epoch-float':
                                bad['epoch']=bad['current_operation']['epoch']=5.0
                            elif fault=='native-kill-id-float': bad['current_operation']['id']=4.0
                            conn.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(bad),))
                        elif fault=='attempt-float':
                            bad=copy.deepcopy(rows[0][0])
                            bad['identity']['attempt']=bad['command']['attempt']=1.0
                            conn.execute('UPDATE outbox SET action_json=? WHERE id=0',(json.dumps(bad),))
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
            binding=dict(owner=2,sandbox=2,generation=1,vm=3,network=10)))
        stamp=dict(path='/fixed/helper',sha256='a'*64,expected_sha256='a'*64)
        faults = ('none','helper-failed','incomplete','observed-success','wrong-successor','wrong-action',
                  'metadata','kernel','sql','guest','reuse','helper-changed')
        for slot_close, fault in product((False, True), faults):
            selected = [slot_rows(rows)[1]] if slot_close else rows
            valid = dict(version=2, context_rejected=True, release_rejected=True,
                occupied_successor=occupant['context'], action_outcomes=[
                    dict(logical_action=action['identity']['logical_action'], attempt=action['identity']['attempt'],
                         outcome=dict(status='unknown', reason='observation_unavailable'))
                    for action, _, _ in selected])
            with self.subTest(slot_close=slot_close,fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                processes=Mock();runner=NS(root=Path(directory),services=NS(processes=processes,
                    wait_ready=Mock(),native_pools=[dict(node_state_dir='original-state',chroot_base='original-jail')],
                    host_configs=[{}]))
                probe_type = late.LateSlotCloseProbe if slot_close else late.LateHostActionsProbe
                probe=probe_type(runner,NS(session='session-A',current_acceptance=resume),
                    prior,resume,stamp,time.monotonic()+10)
                probe._configuration=Mock();probe.recovered=True;probe.actions=[row[0] for row in selected]
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
                if fault=='incomplete':result['action_outcomes'].pop()
                if fault=='observed-success':result['action_outcomes'][0]['outcome']={'status':'observed_success'}
                if fault=='wrong-successor':result['occupied_successor']['binding']['vm']+=1
                if fault=='wrong-action':result['action_outcomes'][-1]['logical_action']='effect-99'
                command=stack.enter_context(patch.object(late,'bounded_json_command',return_value=result,
                    side_effect=ValueError('helper initialization failed') if fault=='helper-failed' else None))
                guest=Mock(return_value={'original':'guest'},side_effect=ValueError('guest changed') if fault=='guest' else None)
                current=copy.deepcopy(occupant)
                if fault=='reuse':current['context']['expected_previous']=1
                if fault=='none':
                    evidence = probe.replay(current,guest)
                    self.assertTrue(evidence['same_kernel_identity'])
                    self.assertEqual(evidence['probe']['action_outcomes'], valid['action_outcomes'])
                    guest.assert_called_once();self.assertEqual(observer.verify.call_count,2)
                    self.assertEqual(command.call_args.args[0][0],stamp['path'])
                    actions_path = Path(directory)/(probe.artifact_prefix+'-actions.json')
                    self.assertEqual(json.loads(actions_path.read_bytes())['actions'],probe.actions)
                    self.assertEqual(tree.call_count,2)
                    with self.assertRaises(ValueError):probe.replay(current,guest)
                else:
                    with self.assertRaises((ValueError,AssertionError)):probe.replay(current,guest)
                    if fault not in ('guest',):guest.assert_not_called()
                    if fault=='reuse':command.assert_not_called();processes.stop.assert_not_called()
                    if fault not in ('guest','reuse'):processes.start.assert_not_called()

    def test_cleanup_slot_replay_requires_fresh_typed_successor_network_before_host_stop(self):
        prior, resume, _, _, _, _, _ = fixture()
        for slot in (9, 8, 10.0, True):
            with self.subTest(slot=slot):
                processes = Mock()
                runner = NS(services=NS(processes=processes, native_pools=[{}], host_configs=[{}]))
                client = NS(session=prior['session'], current_acceptance=resume)
                probe = late.LateSlotCloseProbe(runner, client, prior, resume, {}, time.monotonic()+5)
                probe.recovered = True
                successor = dict(position=0, expected_previous=2,
                                 binding=dict(owner=2, sandbox=2, generation=1, vm=3, network=slot))
                with self.assertRaises(ValueError):
                    probe.replay({'context': successor}, Mock())
                processes.stop.assert_not_called()
                self.assertFalse(probe.replayed)

    def test_bounded_real_child_failures_are_not_refusal_evidence(self):
        env={'PATH':'/usr/bin:/bin','LANG':'C'}
        for code in ('import sys;sys.exit(1)', 'print("not-json")', 'print("x"*1025)',
                     'import time;time.sleep(2)', 'print("{\\"version\\":1,\\"version\\":1}")'):
            with self.subTest(code=code),self.assertRaises((ValueError,TimeoutError)):
                late.bounded_json_command([sys.executable,'-c',code],env,timeout=.1,max_bytes=1024)


class LateStopDeleteSceneTests(unittest.TestCase):
    def test_scene_waits_for_product_gc_before_other_owner_and_preserves_failures(self):
        scenarios = ('slot-reuse-late-stop-delete', 'slot-reuse-late-slot-close')
        for scenario, failed in product(scenarios, (False, True)):
            with self.subTest(scenario=scenario,failed=failed),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
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
                slot_close = scenario == 'slot-reuse-late-slot-close'
                probe_name = 'LateSlotCloseProbe' if slot_close else 'LateHostActionsProbe'
                stack.enter_context(patch.object(late,probe_name,return_value=probe))
                stack.enter_context(patch.object(late,'approved_probe',return_value={}))
                stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
                stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
                stack.enter_context(patch.object(scene,'check_center_configuration',return_value=dict(deployment='original')))
                args=NS(scenario=scenario,scenario_timeout=60,activation_gate_timeout_ms=60000)
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
                    self.assertIn('late_slot_close' if slot_close else 'late_stop_delete', report)
                self.assertLess(events.index('destroy-1'),events.index('late-stop-delete'))
                runner.finish.assert_called_once()

    def test_cli_registers_selector_and_exact_helper_inputs(self):
        result=subprocess.run([sys.executable,scene.__file__,'--help'],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('slot-reuse-late-stop-delete',result.stdout)
        self.assertIn('slot-reuse-late-slot-close',result.stdout)
        self.assertIn('--stale-probe-sha256',result.stdout)
