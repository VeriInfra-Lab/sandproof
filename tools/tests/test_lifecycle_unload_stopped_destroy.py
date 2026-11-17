"""Finite E8 consumers using the original protocol fixture; no VM provider."""
import copy
from contextlib import ExitStack
import hashlib
import json
import os
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
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
import lifecycle_unload_stopped_destroy as stopped
from tools.tests import test_internal_fault_relay as transport


def fixture(root):
    node, create, _ = transport.InternalFaultRelayTests.pair(None)
    namespace = '0123456789abcdef'
    resume = copy.deepcopy(create)
    resume['accepted_version'] = 3
    resume['request']['binding']['key']['sequence'] = 3
    resume['request']['binding']['expected'] = {'state':'version', 'version':2}
    resume['request']['operation'] = dict(kind='resume', source_incarnation=1, capture=3,
        target_incarnation=7, manifest_digest=[7]*32)
    unload = copy.deepcopy(resume)
    unload['accepted_version'] = 4
    unload['request']['binding']['key']['sequence'] = 4
    unload['request']['binding']['expected']['version'] = 3
    unload['request']['operation'] = dict(kind='unload', source_incarnation=7, capture=5)
    destroy = copy.deepcopy(unload)
    destroy['accepted_version'] = 5
    destroy['request']['binding']['key']['sequence'] = 5
    destroy['request']['binding']['expected']['version'] = 4
    destroy['request']['operation'] = {'kind':'destroy'}
    def accepted(intent):
        return dict(session=intent['request']['binding']['session'],
            key=copy.deepcopy(intent['request']['binding']['key']), accepted_version=intent['accepted_version'])
    def entry(intent, operation):
        return dict(intent=copy.deepcopy(intent), local_operation=operation, applied_deadline=None,
                    previous_effective_end_at_ms=None, unload_resolution=None)
    old, terminal = entry(unload,40), entry(destroy,50)
    local = dict(owner=1, sandbox=4, operation=30)
    source = dict(pid=123, start_time_ticks=456, boot_id='boot', resource_id='vm-'+namespace+'-9')
    resource = dict(kind='vm', id=source['resource_id'])
    def action(kind, operation=40, epoch=3, logical=400):
        fields = dict(sandbox=4, generation=2, vm=9)
        if kind == 'StopVm': fields['epoch'] = epoch
        if kind == 'InstallFence': fields = dict(sandbox=4, epoch=epoch, terminal=True)
        return dict(identity=dict(protocol_version=1, owner='tenant-1', sandbox='sb-'+namespace+'-4',
            operation='op-'+namespace+'-4-'+str(operation), epoch=epoch, generation=2,
            logical_action='effect-'+str(logical), attempt=0,
            resource=dict(kind='sandbox',id='sb-'+namespace+'-4') if kind=='InstallFence' else copy.deepcopy(resource)),
            command=dict(attempt=0, logical=dict(command={kind:fields})))
    def receipt(action, kind, value):
        return dict(identity=copy.deepcopy(action['identity']), outcome=dict(status='observed_success',
            details=dict(resource=copy.deepcopy(action['identity']['resource']),
                command=copy.deepcopy(action['command']['logical']['command']), result={kind:value})))
    stop = action('StopVm')
    requirement = dict(source={k:source[k] for k in ('pid','start_time_ticks','boot_id')},
        resource=resource, node=node, work_binding=[8]*32)
    full = dict(state_bytes=1, memory_bytes=2, disk_bytes=3, pause=dict(
        binding=dict(operation=unload['request']['binding'],source_incarnation=7,capture=5),
        source=copy.deepcopy(requirement['source']),source_resource=resource,
        source_work_binding=requirement['work_binding'],node=node))
    captured = dict(operation=40, full=full, reserved_material_bytes=2*6+2*262144+65536,
        material_bytes=6, action={'original':'seal'}, worker={'original':'worker'}, completed=[9]*32,end=None)
    work = dict(work=dict(anchor=dict(local=local,native=dict(generation=2,current_vm=9),
        request=unload,execution_origin=resume),requirement=requirement),capture=captured,
        restore=None,retirement=None,material_release=dict(producer=None,consumer=None),observation=None)
    backing = copy.deepcopy(work)
    first = copy.deepcopy(unload)
    first['accepted_version'] = 2
    first['request']['binding']['key']['sequence'] = 2
    first['request']['binding']['expected']['version'] = 1
    first['request']['operation'] = dict(kind='unload',source_incarnation=1,capture=3)
    backing['work']['anchor']['request'] = first
    backing['work']['anchor']['execution_origin'] = create
    backing['capture']['operation'] = 20
    backing['capture']['full']['pause']['binding'] = dict(
        operation=first['request']['binding'],source_incarnation=1,capture=3)
    backing['capture']['completed'] = [7]*32
    backing['restore'] = dict(intent=resume,material_use=True)
    rows = [backing,work]
    material = {7:dict(material_paths=[str(root/'A'),str(root/'A-complete')],checked='A'),
                9:dict(material_paths=[str(root/'B'),str(root/'B-complete')],checked='B')}
    config = dict(inbox=dict(node=node), native=dict(maintenance=dict(
        capacity=dict(max_sql_rows=64,max_sql_value_bytes=65536))))
    services = NS(native_pools=[dict(namespace=namespace,database=str(root/'state.db'),node_state_dir=str(root/'node'))],
        host_configs=[config],
        processes=Mock(),wait_ready=Mock())
    runner = NS(services=services,material_paths={},gc_complete=False,physical_complete=False,
        _materials=Mock(side_effect=lambda full,captured:copy.deepcopy(material[captured['completed'][0]])))
    client = Mock(spec=scene.PublicUnloadScenario)
    client.session = accepted(unload)['session']; client.current_acceptance = accepted(resume)
    client.destroy_request = dict(window=4,sequence=5,expected_version=4)
    prior = dict(session=client.session,local=local,source=source,running_backing=material[7],
        context=dict(position=1,binding=dict(owner=1,sandbox=4,generation=2,vm=9,network=10)))
    p = stopped.StoppedUnloadDestroyProbe(runner,client,prior)
    client.current_acceptance = accepted(unload)
    p.host=Mock(pid=777);p.host.poll.return_value=86;p.host.wait.return_value=86
    p.deadline=time.monotonic()+30;p.config=dict(sandbox='sb-'+namespace+'-4');p.pidfd=999997;p._configuration=Mock()
    native=dict(owner=1,generation=2,epoch=3,terminal=False)
    snapshot=(old,rows,[(stop,'dispatched',None)])
    def cleanup_snapshot(kind):
        if kind=='unload':
            cleanup=action('CleanupVm',logical=401)
            actions=[(stop,'receipted',receipt(stop,'Stopped',dict(pid=123,start_ticks=456))), (cleanup,'dispatched',None)]
            extra=dict(current_version=4,terminal=None,terminal_actions=[],native=copy.deepcopy(native))
        else:
            cleanup=action('CleanupVm',50,4,503);terminal_stop=action('StopVm',50,4,502);fence=action('InstallFence',50,4,501)
            actions=[(stop,'dispatched',None)]
            extra=dict(current_version=5,terminal=copy.deepcopy(terminal),native=dict(native,epoch=4,terminal=True,
                    current_operation=dict(id=50,kind='Kill',epoch=4,status='Pending')),
                terminal_actions=[(fence,'receipted',receipt(fence,'Fence',dict(epoch=4,terminal=True))),
                    (terminal_stop,'receipted',receipt(terminal_stop,'Stopped',dict(pid=123,start_ticks=456))),
                    (cleanup,'dispatched',None)])
        extra['global_pending_cleanups'] = [cleanup]
        return copy.deepcopy((old,rows,actions,extra))
    return NS(probe=p,runner=runner,client=client,node=node,source=source,old=old,terminal=terminal,
        unload=unload,destroy=destroy,resume=resume,accepted=accepted,stop=stop,snapshot=snapshot,
        cleanup_snapshot=cleanup_snapshot,material=material,receipt=receipt)


class StoppedDestroyFactsTests(unittest.TestCase):
    def before(self,f,stack):
        p=f.probe
        def original_read(probe,deadline,**kwargs):
            return (*f.snapshot,[f.stop]) if kwargs.get('extra') is not None else f.snapshot
        stack.enter_context(patch.object(capture.CaptureHandoffProbe,'_read',original_read))
        stack.enter_context(patch.object(capture,'pidfd_exited',return_value=True))
        stack.enter_context(patch.object(stopped,'pidfd_exited',return_value=True))
        stack.enter_context(patch.object(stopped,'original_process',return_value=False))
        return p.after_acceptance(f.accepted(f.unload),p.deadline)

    def test_original_stop_cut_retains_both_full_materials_and_distinct_native_operation(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));result=self.before(f,stack);p=f.probe
            self.assertTrue(result['stopped_before_destroy']['original_stop_sql_receipt_pending'])
            self.assertEqual(p.action['identity']['operation'],'op-0123456789abcdef-4-40')
            self.assertEqual(p.before_destroy['source_materials'],f.material[9])
            self.assertEqual(p.before_destroy['backing_materials'],f.material[7])
            self.assertEqual(f.runner.material_paths[f.client.session],set(f.material[9]['material_paths']))
            self.assertNotEqual(f.unload['retention']['window'],f.unload['request']['binding']['key']['window'])

    def test_both_cleanup_orders_use_same_original_stop_and_complete_receipt_contract(self):
        for kind in ('unload','kill'):
            with self.subTest(kind=kind),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.before(f,stack)
                stack.enter_context(patch.object(stopped,'original_json',return_value=dict(epoch=4,terminal=True)))
                result=f.probe._cleanup_facts(f.cleanup_snapshot(kind))
                self.assertEqual(result['order'],'old-unload-cleanup' if kind=='unload' else 'terminal-kill-cleanup')
                self.assertTrue(result['cleanup_sql_ack_pending'])
                self.assertFalse(result['capacity_release_inferred'])

    def test_missing_action_fields_wrong_source_or_early_ack_cannot_pass_cleanup(self):
        mutations=[
            lambda s:s[2][0][0]['identity'].update(logical_action='effect-other'),
            lambda s:s[2][1][0]['identity'].update(owner='tenant-other'),
            lambda s:s[2][1][0]['command']['logical']['command']['CleanupVm'].update(vm=10),
            lambda s:s[2].__setitem__(1,(s[2][1][0],'receipted',{})),
            lambda s:s[2][0][2]['outcome']['details']['result']['Stopped'].update(pid=124),
            lambda s:s[3]['native'].update(owner=True),
            lambda s:s[3]['native'].update(epoch=2),
            lambda s:s[3]['global_pending_cleanups'].append(copy.deepcopy(s[3]['global_pending_cleanups'][0])),
            lambda s:s[2][1][0]['identity'].pop('logical_action'),
            lambda s:s[2][1][0]['command']['logical']['command']['CleanupVm'].update(sandbox=True),
        ]
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.before(f,stack)
            for i,mutate in enumerate(mutations):
                snapshot=f.cleanup_snapshot('unload');mutate(snapshot)
                with self.subTest(index=i),self.assertRaises((ValueError,KeyError)):
                    f.probe._cleanup_facts(snapshot)

    def test_terminal_cleanup_requires_the_complete_current_native_kill(self):
        for fault in ('none','operation','kind','epoch','typed-id','missing'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.before(f,stack);snapshot=f.cleanup_snapshot('kill')
                operation=snapshot[3]['native']['current_operation']
                if fault=='operation':operation['id']+=1
                if fault=='kind':operation['kind']='Pause'
                if fault=='epoch':operation['epoch']+=1
                if fault=='typed-id':operation['id']=50.0
                if fault=='missing':snapshot[3]['native']['current_operation']=None
                stack.enter_context(patch.object(stopped,'original_json',return_value=dict(epoch=4,terminal=True)))
                if fault=='none':self.assertEqual(f.probe._cleanup_facts(snapshot)['order'],'terminal-kill-cleanup')
                else:
                    with self.assertRaises(ValueError):f.probe._cleanup_facts(snapshot)
                    self.assertFalse(f.probe.recovered);self.assertIsNone(f.probe.cleanup_cut)

    def test_destroy_binding_and_default_prepare_guard_remain_separate(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.before(f,stack);p=f.probe;f.client.current_acceptance=f.accepted(f.destroy)
            with self.assertRaisesRegex(ValueError,'paused original Prepare'):
                capture.CaptureHandoffProbe._bind_terminal_acceptance(p,f.client.current_acceptance)
            self.assertIsNone(p.terminal_accepted)
            p._bind_next_destroy(f.client.current_acceptance)
            inbox=dict(sessions=[dict(id=f.client.session,current_version=5,terminal=True,
                local=p.prior['local'],entries=[f.old,f.terminal])])
            old,terminal,_=p._terminal_entries_for(inbox,p.original_entry)
            self.assertEqual(old,f.old);self.assertEqual(terminal,f.terminal)
            pending=copy.deepcopy(inbox);pending['sessions'][0]['entries'][1]['local_operation']=None
            with self.assertRaises(capture._AwaitTerminal):p._terminal_entries_for(pending,p.original_entry)
            _,received,_=p._terminal_entries_for(pending,p.original_entry,require_admitted=False)
            self.assertIsNone(received['local_operation'])
            inbox['sessions'][0]['entries'][1]['intent']['center']['center']='other'
            with self.assertRaises(ValueError):p._terminal_entries_for(inbox,p.original_entry)

    def test_kernel_exit_is_required_and_zombie_identity_does_not_imply_resource_release(self):
        with tempfile.TemporaryDirectory() as directory:
            p=fixture(Path(directory)).probe
            for present in (False,True):
                with patch.object(stopped,'pidfd_exited',return_value=False),patch.object(stopped,'original_process',return_value=present),self.assertRaises(ValueError):p._exited()
            with patch.object(stopped,'pidfd_exited',return_value=True),patch.object(stopped,'original_process',return_value=True):
                value=p._exited();self.assertTrue(value['process_identity_present']);self.assertFalse(value['resource_release_inferred'])

    def test_terminal_cut_rejects_missing_fence_wrong_receipt_and_original_ready_rollback(self):
        mutations=[
            lambda s:s[3]['terminal_actions'].__setitem__(0,(s[3]['terminal_actions'][0][0],'dispatched',None)),
            lambda s:s[3]['terminal_actions'][1][2]['outcome']['details']['result']['Stopped'].update(pid=124),
            lambda s:s[3]['terminal_actions'][2][0]['identity'].update(epoch=3),
            lambda s:s[3]['native'].update(terminal=False),
            lambda s:s[2].__setitem__(0,(s[2][0][0],'ready',None)),
        ]
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.before(f,stack)
            stack.enter_context(patch.object(stopped,'original_json',return_value=dict(epoch=4,terminal=True)))
            for i,mutate in enumerate(mutations):
                snapshot=f.cleanup_snapshot('kill');mutate(snapshot)
                with self.subTest(index=i),self.assertRaises((ValueError,KeyError)):
                    f.probe._cleanup_facts(snapshot)

    def test_before_destroy_rejects_changed_full_origin_pin_digest_and_charge(self):
        mutations=[
            lambda s:s[1][1]['work']['anchor']['execution_origin']['request']['binding']['key'].update(owner='2'),
            lambda s:s[1][1]['work']['requirement']['resource'].update(kind='disk'),
            lambda s:s[1][0]['restore'].update(material_use=False),
            lambda s:s[1][0]['capture'].update(completed=[8]*32),
            lambda s:s[1][1]['capture'].update(reserved_material_bytes=0),
        ]
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.before(f,stack)
            for i,mutate in enumerate(mutations):
                snapshot=copy.deepcopy(f.snapshot);mutate(snapshot)
                with self.subTest(index=i),self.assertRaises((ValueError,KeyError)):
                    f.probe._before_facts(snapshot)

    def test_already_recorded_inconsistent_resume_full_pair_is_rejected_without_forcing_plus_one(self):
        for fault in ('target','source','capture'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory))
                operation=f.snapshot[1][1]['work']['anchor']['execution_origin']['request']['operation']
                field={'target':'target_incarnation','source':'source_incarnation','capture':'capture'}[fault]
                operation[field]+=1
                with self.assertRaisesRegex(ValueError,'both Full bindings'):self.before(f,stack)


class RecoverySequenceTests(unittest.TestCase):
    def run_recovery(self,*,kind='unload',fault=None):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));StoppedDestroyFactsTests.before(None,f,stack);p=f.probe
            f.client.current_acceptance=f.accepted(f.destroy)
            cut=f.cleanup_snapshot(kind)
            if fault=='wrong-cut':
                target=cut[2] if kind=='unload' else cut[3]['terminal_actions']
                target[-1][0]['command']['logical']['command']['CleanupVm']['vm']=10
            if fault=='materials':
                f.runner._materials.side_effect=lambda full,captured:dict(checked='changed',material_paths=[])
            p._read=Mock(side_effect=[(*f.snapshot,[f.stop]),cut])
            cleanup_host=Mock(pid=888);cleanup_host.wait.return_value=86
            normal_host=Mock(pid=999)
            f.runner.services.processes.start.side_effect=[cleanup_host,normal_host]
            f.client.assert_terminal_unload_replay.return_value={'phase':'terminating'}
            stack.enter_context(patch.object(stopped,'read_host',return_value={'current_version':4 if kind=='unload' else 5}))
            stack.enter_context(patch.object(stopped,'original_json',return_value=dict(epoch=4,terminal=True)))
            if fault:
                with self.assertRaises(ValueError):p.recover_after_destroy(f.client.current_acceptance,p.deadline)
                self.assertFalse(p.recovered)
                self.assertEqual(f.runner.services.processes.start.call_count,0 if fault=='materials' else 1)
                self.assertEqual(p.terminal_failure_state()['destroy_acceptance'],f.accepted(f.destroy))
            else:
                result=p.recover_after_destroy(f.client.current_acceptance,p.deadline)
                self.assertTrue(result['original_recovery_resumed']);self.assertTrue(p.recovered)
                self.assertEqual(f.runner.services.processes.start.call_args_list[0].kwargs,
                    {'env':{'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'cleanup_vm'}})
                self.assertEqual(len(f.runner.services.processes.start.call_args_list),2)
                f.client.assert_terminal_unload_replay.assert_called_once_with(phase='terminating')
                f.runner.gc_complete=f.runner.physical_complete=True
                self.assertTrue(p.cleanup()['product_gc_complete'])
            with self.assertRaisesRegex(ValueError,'old Unloaded'):p.finish(p.deadline)

    def test_each_legal_cleanup_order_reopens_original_configuration_only_after_the_stable_cut(self):
        for kind in ('unload','kill'):
            with self.subTest(kind=kind):self.run_recovery(kind=kind)

    def test_changed_material_or_wrong_cleanup_preserves_acceptances_without_automatic_followup(self):
        for fault in ('materials','wrong-cut'):
            with self.subTest(fault=fault):self.run_recovery(fault=fault)


class OriginalSqlTests(unittest.TestCase):
    def database(self,f,*,terminal=False):
        p=f.probe;p.accepted=f.accepted(f.unload);p.original_entry=copy.deepcopy(f.old);p.action=copy.deepcopy(f.stop)
        p.terminal_accepted=f.accepted(f.destroy) if terminal else None
        inbox=dict(sessions=[dict(id=f.client.session,current_version=5 if terminal else 4,terminal=terminal,
            local=p.prior['local'],entries=[f.old,f.terminal] if terminal else [f.old])])
        raw=json.dumps(inbox).encode();database=p.runtime.services.native_pools[0]['database']
        cut=f.cleanup_snapshot('kill' if terminal else 'unload')
        rows=[(f.stop,'dispatched',None)] if not terminal else cut[2]+cut[3]['terminal_actions']
        with sqlite3.connect(database) as conn:
            conn.execute('PRAGMA journal_mode=WAL')
            conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);'
                'CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB);'
                'CREATE TABLE controller_effect_keys(id INTEGER,logical_json BLOB);'
                'CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB,owner_id TEXT);')
            conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,65536)',(raw,hashlib.sha256(raw).digest()))
            for i,(action,state,receipt) in enumerate(rows,1):
                conn.execute('INSERT INTO controller_effect_keys VALUES(?,?)',
                    (int(action['identity']['logical_action'].removeprefix('effect-')),json.dumps(action['command']['logical']).encode()))
                conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?)',(i,action['identity']['sandbox'],action['identity']['operation'],
                    json.dumps(action).encode(),state,None if receipt is None else json.dumps(receipt).encode()))
            conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',(p.config['sandbox'],json.dumps(cut[3]['native']).encode(),'tenant-1'))
        return database

    def test_exact_current_default_is_preserved_and_explicit_destroy_reads_same_old_entry(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.database(f,terminal=True);p=f.probe
            stack.enter_context(patch.object(capture,'private_path',side_effect=lambda value,**_:Path(value)))
            stack.enter_context(patch.object(capture,'checked_source_records',return_value=f.snapshot[1]))
            with self.assertRaisesRegex(ValueError,'exact current accepted'):
                p._read(p.deadline)
            value=p._read(p.deadline,current_version=5,extra=p._extra)
            self.assertEqual(value[0],f.old);self.assertEqual(value[3]['terminal'],f.terminal)
            self.assertEqual(len(value[3]['terminal_actions']),3)
            self.assertEqual(value[3]['native']['epoch'],4)

    def test_explicit_and_persisted_current_versions_keep_integer_identity(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));database=self.database(f,terminal=True);p=f.probe
            stack.enter_context(patch.object(capture,'private_path',side_effect=lambda value,**_:Path(value)))
            records=stack.enter_context(patch.object(capture,'checked_source_records',return_value=f.snapshot[1]))
            self.assertEqual(p._read(p.deadline,current_version=5,extra=p._extra)[0],f.old)
            for version in (5.0,True,0,-1,2**64):
                records.reset_mock()
                with self.subTest(version=version),self.assertRaisesRegex(ValueError,'one exact current version'):
                    p._read(p.deadline,current_version=version,extra=p._extra)
                records.assert_not_called()
            with sqlite3.connect(database) as conn:
                raw,=conn.execute('SELECT state_json FROM controller_lifecycle_inbox WHERE id=1').fetchone()
                inbox=json.loads(raw);inbox['sessions'][0]['current_version']=5.0
                raw=json.dumps(inbox).encode()
                conn.execute('UPDATE controller_lifecycle_inbox SET state_json=?,sha256=? WHERE id=1',
                    (raw,hashlib.sha256(raw).digest()))
            records.reset_mock()
            with self.assertRaisesRegex(ValueError,'exact current accepted'):
                p._read(p.deadline,current_version=5,extra=p._extra)
            records.assert_not_called()

    def test_native_and_current_intent_extension_use_the_original_sql_snapshot(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));database=self.database(f);p=f.probe;writes=[]
            def records(conn,*_):
                if not writes:
                    changed=dict(owner=2,generation=2,epoch=3,terminal=False)
                    with sqlite3.connect(database) as other:
                        other.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(changed).encode(),))
                    writes.append(True)
                return f.snapshot[1]
            stack.enter_context(patch.object(capture,'private_path',side_effect=lambda value,**_:Path(value)))
            stack.enter_context(patch.object(capture,'checked_source_records',side_effect=records))
            value=p._read(p.deadline,extra=p._extra)
            self.assertEqual(value[3]['native']['owner'],1)
            self.assertEqual(len(p._read(p.deadline)),3)
            with self.assertRaisesRegex(ValueError,'native terminal owner'):
                p._read(p.deadline,extra=p._extra)

    def test_another_global_stop_cannot_be_mistaken_for_the_selected_success_cut(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));database=self.database(f);p=f.probe
            stack.enter_context(patch.object(capture,'private_path',side_effect=lambda value,**_:Path(value)))
            stack.enter_context(patch.object(capture,'checked_source_records',return_value=f.snapshot[1]))
            self.assertEqual(len(p._before_snapshot(p.deadline)),3)
            other=copy.deepcopy(f.stop)
            other['identity'].update(owner='tenant-2',sandbox='sb-0123456789abcdef-5',operation='op-0123456789abcdef-5-40',
                generation=1,epoch=6,logical_action='effect-404',resource=dict(kind='vm',id='vm-0123456789abcdef-10'))
            other['command']['logical']['command']['StopVm'].update(sandbox=5,generation=1,epoch=6,vm=10)
            with sqlite3.connect(database) as conn:
                conn.execute('INSERT INTO controller_effect_keys VALUES(404,?)',(json.dumps(other['command']['logical']).encode(),))
                conn.execute('INSERT INTO outbox VALUES(2,?,?,?,\'dispatched\',NULL)',
                    (other['identity']['sandbox'],other['identity']['operation'],json.dumps(other).encode()))
            with self.assertRaisesRegex(ValueError,'one global pending original Stop'):
                p._before_snapshot(p.deadline)

    def test_logical_action_registration_cannot_name_another_vm_under_the_same_key(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));database=self.database(f);p=f.probe
            stack.enter_context(patch.object(capture,'private_path',side_effect=lambda value,**_:Path(value)))
            stack.enter_context(patch.object(capture,'checked_source_records',return_value=f.snapshot[1]))
            p._before_snapshot(p.deadline)
            wrong=copy.deepcopy(f.stop['command']['logical']);wrong['command']['StopVm']['vm']=10
            with sqlite3.connect(database) as conn:
                conn.execute('UPDATE controller_effect_keys SET logical_json=? WHERE id=400',(json.dumps(wrong).encode(),))
            with self.assertRaisesRegex(ValueError,'differs from native registration'):
                p._before_snapshot(p.deadline)


class LinuxExitTests(unittest.TestCase):
    def test_original_pidfd_requires_kernel_exit_and_never_sends_a_signal(self):
        with tempfile.TemporaryDirectory() as directory:
            p=fixture(Path(directory)).probe
            child=subprocess.Popen([sys.executable,'-c','import sys;sys.stdin.buffer.read(1)'],stdin=subprocess.PIPE)
            fd=None
            try:
                fields=(Path('/proc')/str(child.pid)/'stat').read_text().rsplit(')',1)[1].split()
                p.prior['source']=dict(resource_id='vm-controlled-child',pid=child.pid,start_time_ticks=int(fields[19]),
                    boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
                fd=os.pidfd_open(child.pid);p.pidfd=fd
                with self.assertRaises(ValueError):p._exited()
                child.stdin.close();self.assertEqual(child.wait(timeout=5),0)
                result=p._exited();self.assertTrue(result['pidfd_exit_observed'])
                self.assertFalse(result['process_identity_present']);self.assertFalse(result['resource_release_inferred'])
            finally:
                if child.poll() is None:
                    child.stdin.close();child.wait(timeout=5)
                if fd is not None:os.close(fd)


class OriginalScenarioTests(unittest.TestCase):
    def exercise(self,*,failure=False,real_peer=False,peer_fault=None):
        events=[];clients={};lifetime=dict(started_at_ms=1,end_at_ms=2,max_end_at_ms=3,startup_deadline_ms=2)
        for owner in ('1','2'):
            client=Mock(spec=scene.PublicUnloadScenario);client.owner=owner;client.session='session-'+owner
            client.create_acceptance={};client.current_acceptance={};client.unload_acceptance={'key':{}}
            client.concurrent_unload_replies=[{},{}];client.resume_request={};client.witness={}
            client.timings=[{'sent_monotonic':time.monotonic()}];client.last_http_status=None;client.failure_step='controlled'
            client.timing_report.return_value=[]
            client.wait_running.return_value={'phase':'running'}
            client.wait_unloaded.return_value={'phase':'unloaded'}
            client.query.return_value=({'phase':'unloading','unload_outcome':{'state':'pending'}} if owner=='1' else {'phase':'running'})
            client.assert_terminal_unload_replay.side_effect=lambda *,phase:dict(phase=phase)
            for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_resumed_workload','assert_running_workload'):
                getattr(client,name).side_effect=lambda *_,n=name,o=owner,**__:events.append(n+'-'+o) or {}
            clients[owner]=client
        if real_peer:
            peer = clients['2']
            accepted = dict(session=peer.session, key=dict(deployment='original',owner='2',window=1,sequence=1),accepted_version=1)
            peer.current_acceptance = peer.create_acceptance = accepted
            peer.unload_acceptance = None
            peer.wait_running.return_value = dict(session=peer.session,operation=accepted['key'],current_version=1,
                phase='running',unload_outcome=None,create_refusal=None)
            peer.query.return_value = peer.wait_running.return_value
            state = {}
            def initialize_peer(*_):
                events.append('run_public_workload-2')
                state.update(nonce='a'*64,activation_nonce='b'*64,metadata_generation=1,counter=1,pid=41,start_time_ticks=7)
                peer.witness = copy.deepcopy(state);peer.data_token = 'original-peer-token'
                return {}
            peer.run_public_workload.side_effect = initialize_peer
            sdk = Mock();sdk.files.read.side_effect = lambda _: 'disk-counter:'+str(state['counter'])
            def sdk_provider():
                if peer_fault=='authorization':peer.data_token='another-execution'
                return sdk
            peer.sdk.side_effect = sdk_provider
            def witness(_, command):
                if command == 'advance' and peer_fault!='counter':state['counter'] += 1
                return copy.deepcopy(state)
            peer.witness_command.side_effect = witness
            for method in ('check_view','_advance_same_running_workload'):
                setattr(peer,method,getattr(scene.PublicUnloadScenario,method).__get__(peer))
            check_running = scene.PublicUnloadScenario.assert_running_workload.__get__(peer)
            def observe_running(*args):
                events.append('assert_running_workload-2')
                return check_running(*args)
            peer.assert_running_workload.side_effect = observe_running
        calls=0
        def running(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context={'position':0 if session=='session-2' or calls==1 else 1})
        probe=Mock(spec=stopped.StoppedUnloadDestroyProbe)
        for name in ('arm','after_acceptance','recover_after_destroy','cleanup','close'):
            getattr(probe,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        if failure:
            def unknown(*_):events.append('recover_after_destroy');raise ValueError('unknown original cleanup cut')
            probe.recover_after_destroy.side_effect=unknown
        probe.terminal_failure_state.return_value={'original_acceptance':'preserved','capacity_release_inferred':False}
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='original',foreign_key='peer',services=NS(sdk_url='http://127.0.0.1:1'),
                measurement_identity=lambda:{},start=lambda _:None,create_parameters=lambda **_:{},record_running=Mock(side_effect=running),
                unload_evidence=Mock(return_value=dict(facts=lifetime,original_position={'position':0},original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation':{}}),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),
                finish=Mock(side_effect=lambda _:events.append('runtime-finish')),fail_resume_gate=Mock(return_value='unarmed'),
                close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            constructor=stack.enter_context(patch.object(stopped,'StoppedUnloadDestroyProbe',return_value=probe))
            args=NS(scenario='capture-stopped-destroy',scenario_timeout=60,activation_gate_timeout_ms=60000)
            if peer_fault is not None:
                with self.assertRaises(AssertionError):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_text())
                self.assertFalse(report['complete']);self.assertEqual(len(report['known_sessions']),2)
                self.assertEqual(report['phase'],'other-owner-progress-after-stopped-destroy')
                self.assertIn('stopped_destroy_failure_state',report)
                self.assertIn('recover_after_destroy',events)
                clients['2'].destroy.assert_not_called();probe.cleanup.assert_not_called()
                runner.settlement_evidence.assert_not_called()
            elif failure:
                with self.assertRaisesRegex(ValueError,'unknown original cleanup cut'):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_text())
                self.assertFalse(report['complete']);self.assertEqual(len(report['known_sessions']),2)
                self.assertIn('stopped_destroy_failure_state',report)
                probe.cleanup.assert_not_called();runner.settlement_evidence.assert_not_called()
                clients['2'].destroy.assert_not_called()
            else:
                report=scene.exercise(args)
                self.assertTrue(report['complete']);self.assertEqual(len(report['cycles']),1)
                self.assertTrue(report['cycles'][0]['stopped_unload_destroyed'])
                self.assertIn('E8',report['acceptance_cases']);self.assertEqual(clients['1'].resume.call_count,1)
                self.assertLess(events.index('recover_after_destroy'),events.index('assert_running_workload-2'))
                self.assertLess(events.index('recover_after_destroy'),events.index('destroy-2'))
                self.assertLess(events.index('run_public_workload-2'), events.index('arm'))
                self.assertLess(events.index('run_public_workload-2'), events.index('assert_running_workload-2'))
                clients['2'].assert_continued_workload.assert_not_called()
                clients['2'].destroy.assert_called_once();probe.cleanup.assert_called_once()
            self.assertEqual(clients['1'].unload.call_count,2);clients['1'].destroy.assert_called_once()
            probe.finish.assert_not_called();constructor.assert_called_once();runner.finish.assert_called_once()
            self.assertLess(events.index('runtime-finish'),events.index('close'))

    def test_actual_scene_preserves_peer_until_original_destroy_recovery_and_finishes_product_gc(self):
        self.exercise()

    def test_original_scene_calls_real_running_checker_after_peer_workload_setup(self):
        self.exercise(real_peer=True)

    def test_unknown_cleanup_keeps_both_acceptances_without_retry_or_peer_destroy(self):
        self.exercise(failure=True)


    def test_stalled_original_peer_keeps_its_session_after_first_owner_cleanup(self):
        self.exercise(real_peer=True,peer_fault='counter')

    def test_changed_original_peer_authorization_cannot_trigger_peer_destroy(self):
        self.exercise(real_peer=True,peer_fault='authorization')


if __name__=='__main__':
    unittest.main()
