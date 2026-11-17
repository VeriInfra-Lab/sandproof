"""Original E8 consumers with controlled facts and real SQLite; never KVM."""
import copy
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import sqlite3
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
import lifecycle_unload_sync_destroy as sync
from tools.tests import test_internal_fault_relay as transport


def fixture(root, stage='sync-before'):
    node, create, _ = transport.InternalFaultRelayTests.pair(None)
    namespace = '0123456789abcdef'
    def intent(version, operation, expected):
        value = copy.deepcopy(create); value['accepted_version'] = version
        value['request']['binding']['key']['sequence'] = version
        value['request']['binding']['expected'] = dict(state='version', version=expected)
        value['request']['operation'] = operation
        return value
    first = intent(2, dict(kind='unload', source_incarnation=1, capture=3), 1)
    resume = intent(3, dict(kind='resume', source_incarnation=1, capture=3,
                           target_incarnation=7, manifest_digest=[7]*32), 2)
    unload = intent(4, dict(kind='unload', source_incarnation=7, capture=5), 3)
    destroy = intent(5, dict(kind='destroy'), 4)
    def accepted(value):
        return dict(session=value['request']['binding']['session'], key=copy.deepcopy(value['request']['binding']['key']),
                    accepted_version=value['accepted_version'])
    def entry(value, operation):
        return dict(intent=copy.deepcopy(value), local_operation=operation, applied_deadline=None,
                    previous_effective_end_at_ms=None, unload_resolution=None)
    old, terminal = entry(unload, 40), entry(destroy, 50)
    local = dict(owner=1, sandbox=4, operation=10)
    source = dict(pid=123, start_time_ticks=456, boot_id='fixture-boot', resource_id='vm-'+namespace+'-9')
    resource = dict(kind='vm', id=source['resource_id'])
    def action(kind, operation=50, epoch=4, logical=501):
        fields = dict(sandbox=4, generation=2, vm=9)
        if kind in ('StopVm', 'SealSnapshot'): fields['epoch'] = epoch
        if kind == 'SealSnapshot': fields.update(workspace=11, snapshot=12, manifest=13)
        if kind == 'InstallFence': fields = dict(sandbox=4, epoch=epoch, terminal=True)
        return dict(identity=dict(protocol_version=1, owner='tenant-1', sandbox='sb-'+namespace+'-4',
            operation='op-'+namespace+'-4-'+str(operation), epoch=epoch, generation=2,
            logical_action='effect-'+str(logical), attempt=0,
            resource=(dict(kind='sandbox',id='sb-'+namespace+'-4') if kind=='InstallFence' else
                      dict(kind='snapshot',id='snapshot-'+namespace+'-12') if kind=='SealSnapshot' else copy.deepcopy(resource))),
            command=dict(attempt=0, logical=dict(command={kind:fields})))
    def receipt(value, kind, result):
        return dict(identity=copy.deepcopy(value['identity']), outcome=dict(status='observed_success', details=dict(
            resource=copy.deepcopy(value['identity']['resource']), command=copy.deepcopy(value['command']['logical']['command']),
            result={kind:result})))
    seal = action('SealSnapshot', 40, 3, 400)
    worker = dict(process=dict(pid=234, start_time_ticks=567, boot_id='fixture-boot'), job_digest=[6]*32)
    process = {key:source[key] for key in ('pid', 'start_time_ticks', 'boot_id')}
    requirement = dict(source=process, resource=resource, node=node, work_binding=[8]*32)
    full = dict(state_bytes=1, memory_bytes=2, disk_bytes=3, pause=dict(
        binding=dict(operation=unload['request']['binding'], source_incarnation=7, capture=5),
        source=copy.deepcopy(process), source_resource=resource, source_work_binding=[8]*32, node=node))
    captured = dict(operation=40, full=full, reserved_material_bytes=2*6+2*262144+65536,
                    material_bytes=None, action=seal, worker=worker, completed=None, end=None)
    work = dict(work=dict(anchor=dict(local=local, native=dict(generation=2, current_vm=9),
        request=unload, execution_origin=resume), requirement=requirement), capture=captured,
        restore=None, retirement=None, material_release=dict(producer=None, consumer=None), observation=None)
    backing = copy.deepcopy(work)
    backing['work']['anchor'].update(request=first, execution_origin=create)
    backing['capture']['full']['pause']['binding'] = dict(operation=first['request']['binding'], source_incarnation=1, capture=3)
    backing['capture'].update(operation=20, completed=[7]*32, material_bytes=6)
    backing['restore'] = dict(intent=copy.deepcopy(resume), material_use=True)
    materials = dict(material_paths=[str(root/'A')], original='A')
    config = dict(inbox=dict(node=node), native=dict(maintenance=dict(capacity=dict(max_sql_rows=64, max_sql_value_bytes=65536))))
    services = NS(native_pools=[dict(namespace=namespace, database=str(root/'state.db'), node_state_dir=str(root/'node'))],
                  host_configs=[config], processes=Mock(), wait_ready=Mock())
    runner = NS(services=services, captures=[dict(layout_parent=str(root))],
                _materials=Mock(return_value=copy.deepcopy(materials)), gc_complete=False, physical_complete=False)
    client = Mock(spec=scene.PublicUnloadScenario); client.session = accepted(unload)['session']
    client.current_acceptance = accepted(resume)
    prior = dict(session=client.session, local=local, source=source, running_backing=materials,
        context=dict(position=1, expected_previous=None, binding=dict(owner=1,sandbox=4,generation=2,vm=9,network=10)))
    p = sync.SyncDestroyProbe(runner, client, prior, stage, root/'capture-sync-gate')
    p.config = dict(schema_version=3, run_identity='a'*64, sandbox='sb-'+namespace+'-4')
    p.accepted = accepted(unload); p.original_entry = old; p.record = copy.deepcopy(work)
    p.action = copy.deepcopy(seal); p.deadline = time.monotonic()+30
    p.host = Mock(pid=777); p.host.poll.return_value = None; p.host.wait.return_value = 86
    p.pidfd, p.worker_pidfd, p.fd = 999997, 999998, 999999
    p.question = dict(schema_version=3, run_identity='a'*64, phase=capture.SYNC_PHASES[stage], capture_key='d'*64,
                      action=copy.deepcopy(seal), worker=worker, source=process, full_work_binding=[9]*32)
    p.question_digest = 'c'*64
    p.sync_reserved_bytes = captured['reserved_material_bytes']; p.sync_material = dict(original_memory=True)
    p._sync_material = Mock(return_value=copy.deepcopy(p.sync_material)); p._configuration = Mock(); p._gate_unchanged = Mock()
    p.slot_gate = dict(path=str(root/'node/helper-gates/1.lock'),
        gate=dict(path=str(root/'node/helper-gates/1.lock'),device=7,inode=11,max_data_routes_per_slot=2),
        file=dict(bytes=8, sha256=[5]*32, device=7, inode=11))
    p._slot_gate = Mock(return_value=copy.deepcopy(p.slot_gate))
    client.current_acceptance = accepted(destroy); client.destroy_request = dict(window=4,sequence=5,expected_version=4)
    p._bind_next_destroy(client.current_acceptance)
    native = dict(owner=1,generation=2,epoch=4,terminal=True,desired='Closed',effective='Closed',current_vm=9,net_slot=10,
                  current_operation=dict(id=50,epoch=4,kind='Kill',status='Pending'))
    fence, stop, cleanup = action('InstallFence'), action('StopVm', logical=502), action('CleanupVm', logical=503)
    old_actions = [(seal,'dispatched',None)]
    terminal_actions = [(fence,'receipted',receipt(fence,'Fence',dict(epoch=4,terminal=True))),
                        (stop,'receipted',receipt(stop,'Stopped',dict(pid=123,start_ticks=456))), (cleanup,'dispatched',None)]
    held = dict(original=old,terminal=terminal,local=local,native=native,sources=[backing,work],
                old_actions=old_actions,terminal_actions=[],global_cleanups=[],action_ids=[(40,seal)])
    cut = copy.deepcopy(held); cut['terminal_actions'] = terminal_actions; cut['global_cleanups'] = [cleanup]
    cut['action_ids'].extend([(51,fence),(52,stop),(53,cleanup)])
    returned = dict(action=copy.deepcopy(seal),full_work_binding=[9]*32,worker=copy.deepcopy(worker),source=copy.deepcopy(process),
                    outcome=dict(kind='returned',http_status=500))
    end = dict(state='terminated',stop=dict(operation=dict(sandbox=4,operation=50),epoch=4,attempt=0,result=52),
        observation=dict(state='stopped_and_drained',work_binding=[4]*32,full_work_binding=[9]*32,
            seal_action=copy.deepcopy(seal),stop_action=copy.deepcopy(stop),source=copy.deepcopy(process),worker=copy.deepcopy(worker),
            host_slot=copy.deepcopy(prior['context']),helper_gate_device=7,helper_gate_inode=11),
        prior=dict(state='worker_ended',observation=returned))
    cut['sources'][1]['capture']['end'] = end
    audit = dict(schema_version=2,node=node,operations=[dict(binding=copy.deepcopy(destroy['request']['binding']),
        accepted_version=5,current=True,terminal=True,local=local,local_operation=50)])
    services.processes.audit.return_value = audit
    services.processes.start.return_value = NS(pid=888)
    return NS(probe=p,held=held,cut=cut,source=source,seal=seal,stop=stop,cleanup=cleanup,old=old,terminal=terminal,
              runner=runner,client=client,node=node,accepted=accepted,unload=unload,destroy=destroy,resume=resume,
              action=action,receipt=receipt,root=root,audit=audit)


def change(value, path, replacement):
    target = value
    for key in path[:-1]: target = target[key]
    target[path[-1]] = replacement


class SyncDestroyFactsTests(unittest.TestCase):
    def facts(self, f, stack, *, exited=False):
        stack.enter_context(patch.object(sync, 'pidfd_exited', return_value=exited))
        stack.enter_context(patch.object(sync, 'original_process', return_value=not exited))
        stack.enter_context(patch.object(sync, 'original_json', return_value=dict(epoch=4,terminal=True)))
        stack.enter_context(patch.object(sync, 'checked_file', return_value=f.probe.slot_gate['file']))

    def test_both_held_sync_boundaries_keep_full_backing_and_closed_native_destroy(self):
        for stage in capture.SYNC_PHASES:
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                f=fixture(Path(directory),stage);self.facts(f,stack)
                result=f.probe._held_facts(f.held)
                self.assertTrue(result['full_worker_still_live']);self.assertTrue(result['backing_pin_retained'])
                self.assertFalse(result['terminal_fence_inferred']);self.assertFalse(result['capacity_released'])
                self.assertEqual(f.resume['request']['operation']['target_incarnation'],7)  # Holes are legal.
                self.assertNotEqual(f.unload['request']['binding']['key']['window'],f.unload['retention']['window'])

    def test_held_cut_rejects_missing_full_pin_charge_origin_or_changed_memory(self):
        cases=[(['sources',1,'capture','completed'],[1]*32),(['sources',1,'capture','end'],{}),
               (['sources',0,'restore','material_use'],False),(['sources',1,'capture','reserved_material_bytes'],1),
               (['sources',1,'work','anchor','execution_origin','request','operation','target_incarnation'],8),
               (['native','desired'],'Running'),(['native','effective'],'Running'),(['native','terminal'],False),
               (['native','current_vm'],10),(['native','net_slot'],11)]
        for path,value in cases:
            with self.subTest(path=path),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.facts(f,stack);change(f.held,path,value)
                with self.assertRaises(ValueError):f.probe._held_facts(f.held)
        for name in ('_sync_material','_slot_gate'):
            with self.subTest(reader=name),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.facts(f,stack);getattr(f.probe,name).return_value={}
                with self.assertRaises(ValueError):f.probe._held_facts(f.held)

    def test_terminal_cut_preserves_actual_returned_end_and_complete_stop_coverage(self):
        for stage in capture.SYNC_PHASES:
            with self.subTest(stage=stage),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory),stage);self.facts(f,stack,exited=True)
                result=f.probe._cut_facts(f.cut)
                self.assertEqual(result['original_end']['prior']['observation']['outcome'],dict(kind='returned',http_status=500))
                self.assertEqual(result['original_end']['stop']['result'],52)
                self.assertTrue(result['cleanup_sql_ack_pending']);self.assertFalse(result['capacity_released'])
                self.assertEqual(f.seal['identity']['resource']['kind'],'snapshot')
                self.assertEqual(f.stop['identity']['resource']['kind'],'vm')

    def test_snapshot_seal_must_not_be_relabelled_as_vm_or_a_foreign_snapshot(self):
        for resource in (dict(kind='vm',id='vm-0123456789abcdef-9'),
                         dict(kind='snapshot',id='snapshot-0123456789abcdef-13')):
            with self.subTest(resource=resource),tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));f.probe.action['identity']['resource']=resource
                with self.assertRaises(ValueError):f.probe._capture_resources(f.held['sources'][1])

    def test_terminal_source_coordinates_attempts_resources_and_forbidden_commands(self):
        for kind in ('fence','stop','cleanup'):
            index={'fence':0,'stop':1,'cleanup':2}[kind]
            for key,value in [('owner','other'),('sandbox','other'),('operation','other'),('epoch',9),
                              ('generation',3),('protocol_version',4),('attempt',1),('resource',dict(kind='vm',id='other'))]:
                with self.subTest(kind=kind,key=key),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                    f=fixture(Path(directory));self.facts(f,stack,exited=True)
                    f.cut['terminal_actions'][index][0]['identity'][key]=value
                    with self.assertRaises(ValueError):f.probe._cut_facts(f.cut)
        for command in ('CommitSnapshot','PrepareVm','ActivateVm'):
            with self.subTest(command=command),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.facts(f,stack,exited=True)
                f.cut['terminal_actions'].append((dict(command=dict(logical=dict(command={command:{}}))),'ready',None))
                with self.assertRaises(ValueError):f.probe._cut_facts(f.cut)

    def test_end_rejects_no_request_wrong_full_worker_source_slot_and_gate(self):
        paths=[(['state'],'worker_ended'),(['stop','result'],51),(['stop','attempt'],1),
               (['observation','full_work_binding'],[8]*32),(['observation','worker','job_digest'],[0]*32),
               (['observation','source','pid'],124),(['observation','host_slot','binding','vm'],10),
               (['observation','helper_gate_inode'],12),(['observation','helper_gate_device'],8),
               (['prior','observation','outcome'],dict(kind='no_request')),
               (['prior','observation','full_work_binding'],[8]*32),(['prior','observation','action'],{})]
        for path,value in paths:
            with self.subTest(path=path),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.facts(f,stack,exited=True)
                change(f.cut['sources'][1]['capture']['end'],path,value)
                with self.assertRaises(ValueError):f.probe._cut_facts(f.cut)

    def test_unknown_receipt_extra_cleanup_or_missing_kernel_exit_cannot_finish(self):
        for fault in ('unknown-stop','wrong-fence','extra-cleanup','live-worker'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));self.facts(f,stack,exited=fault!='live-worker')
                if fault=='unknown-stop':f.cut['terminal_actions'][1][2]['outcome']={'status':'unknown','details':{}}
                if fault=='wrong-fence':f.cut['terminal_actions'][0][2]['outcome']['details']['result']['Fence']['terminal']=False
                if fault=='extra-cleanup':f.cut['global_cleanups'].append(f.action('CleanupVm',logical=504))
                with self.assertRaises(ValueError):f.probe._cut_facts(f.cut)

    def test_original_rust_audit_must_name_complete_current_destroy(self):
        for fault in ('none','key','version','terminal','operation','node'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;p.terminal_entry=f.terminal
                if fault=='key':f.audit['operations'][0]['binding']['key']['sequence']=6
                if fault=='version':f.audit['operations'][0]['accepted_version']=4
                if fault=='terminal':f.audit['operations'][0]['terminal']=False
                if fault=='operation':f.audit['operations'][0]['local_operation']=51
                if fault=='node':f.audit['node']={}
                if fault=='none':self.assertTrue(p._audit_cut()['current_destroy_binding_matched'])
                else:
                    with self.assertRaises(ValueError):p._audit_cut()

    def test_protected_slot_gate_identity_is_read_before_destroy_and_not_guessed_from_end(self):
        for fault in ('none','path','inode','context','boolean-position'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe
                record=dict(position=1,helper_gate=copy.deepcopy(p.slot_gate['gate']),
                            value=dict(context=copy.deepcopy(p.prior['context']),phase='Occupied',retirement=None))
                if fault=='path':record['helper_gate']['path']+='-other'
                if fault=='inode':record['helper_gate']['inode']+=1
                if fault=='context':record['value']['context']['binding']['vm']+=1
                if fault=='boolean-position':record['position']=True
                stack.enter_context(patch.object(sync,'original_json',return_value=record))
                stack.enter_context(patch.object(sync,'checked_file',return_value=p.slot_gate['file']))
                if fault=='none':self.assertEqual(sync.SyncDestroyProbe._slot_gate(p),p.slot_gate)
                else:
                    with self.assertRaises(ValueError):sync.SyncDestroyProbe._slot_gate(p)

    def test_real_arm_keeps_original_sync_variable_and_adds_only_existing_cleanup_cut(self):
        import _center_host_services as services
        for stage in capture.SYNC_PHASES:
            with self.subTest(stage=stage),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                root=Path(directory);f=fixture(root,stage)
                f.runner.quota=dict(deployment_control=str(root));f.runner.lease=NS(identity='a'*64)
                p=sync.SyncDestroyProbe(f.runner,f.client,f.probe.prior,stage,root/'capture-sync-gate')
                p._configuration=Mock();p._save_file=Mock()
                stack.enter_context(patch.object(capture,'open_original_candidate_pidfd',return_value=None))
                try:
                    p.arm(time.monotonic()+5,1000)
                    actual=f.runner.services.processes.start.call_args.kwargs['env']
                    self.assertEqual(services.fault_environment(actual),dict(
                        SANDPROOF_TEST_CAPTURE_SYNC_GATE=str(root/'capture-sync-gate'),
                        SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION='cleanup_vm'))
                    f.runner.services.processes.stop.assert_called_once_with('host-0')
                    self.assertEqual(json.loads((root/'capture-sync-gate/config.json').read_bytes())['sync_phase'],
                                     'before-sync' if stage=='sync-before' else 'after-sync')
                finally:p.close()

    def test_original_sync_handoff_records_only_its_actual_staging_and_publication_paths(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));p=f.probe
            stack.enter_context(patch.object(capture.CaptureHandoffProbe,'after_acceptance',return_value=dict(parent_cut_observed=True)))
            p._read=Mock(return_value=(f.old,f.held['sources'],f.held['old_actions']))
            value=p.after_acceptance(f.accepted(f.unload),p.deadline)
            self.assertTrue(value['parent_cut_observed'])
            self.assertEqual(p.incomplete_material_paths,[str(Path(directory)/'s'/(prefix+'d'*64))
                                                          for prefix in ('staging-','capture-','completed-')])

    def test_gate_retirement_waits_for_product_settlement_and_incomplete_material_absence(self):
        for fault in ('none','unsettled','remaining-staging','dangling-publication'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe;p.recovered=True
                f.runner.gc_complete=f.runner.physical_complete=fault!='unsettled'
                p.incomplete_material_paths=[str(Path(directory)/'staging-original'),str(Path(directory)/'capture-original')]
                if fault=='remaining-staging':Path(p.incomplete_material_paths[0]).mkdir()
                if fault=='dangling-publication':Path(p.incomplete_material_paths[1]).symlink_to(Path(directory)/'absent')
                parent=stack.enter_context(patch.object(capture.CaptureHandoffProbe,'cleanup',return_value=dict(original_gate_archived=True)))
                if fault=='none':self.assertEqual(p.cleanup()['incomplete_material_paths_absent'],p.incomplete_material_paths)
                else:
                    with self.assertRaises(ValueError):p.cleanup()
                    parent.assert_not_called()


class SyncDestroySqlTests(unittest.TestCase):
    def database(self, f, *, cut=True):
        value=f.cut if cut else f.held
        database=Path(f.runner.services.native_pools[0]['database'])
        with sqlite3.connect(database) as conn:
            conn.execute('PRAGMA journal_mode=WAL')
            conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);'
                'CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,state_blob BLOB,owner_id TEXT);'
                'CREATE TABLE outbox(id INTEGER PRIMARY KEY,sandbox_id TEXT,operation_id TEXT,logical_action_id TEXT,attempt INTEGER,action_json BLOB,dispatch_state TEXT,receipt_json BLOB);'
                'CREATE TABLE controller_effect_keys(id INTEGER PRIMARY KEY,logical_json BLOB);')
            inbox=dict(sessions=[dict(id=f.client.session,current_version=5,terminal=True,local=f.held['local'],
                                     entries=[f.old,f.terminal])])
            raw=json.dumps(inbox).encode()
            conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',(raw,hashlib.sha256(raw).digest(),65536))
            conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',(f.probe.config['sandbox'],json.dumps(value['native']).encode(),'tenant-1'))
            for action,state,receipt in value['old_actions']+value['terminal_actions']:
                logical=int(action['identity']['logical_action'].removeprefix('effect-'))
                rowid={400:40,501:51,502:52,503:53}[logical]
                conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?,?,?)',(rowid,action['identity']['sandbox'],
                    action['identity']['operation'],action['identity']['logical_action'],action['identity']['attempt'],
                    json.dumps(action).encode(),state,None if receipt is None else json.dumps(receipt).encode()))
                conn.execute('INSERT INTO controller_effect_keys VALUES(?,?)',(logical,json.dumps(action['command']['logical']).encode()))
        database.chmod(0o600)
        return database

    def test_held_destroy_can_be_admitted_without_scheduling_terminal_actions(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.database(f,cut=False)
            stack.enter_context(patch.object(sync,'checked_source_records',return_value=f.held['sources']))
            value=f.probe._snapshot(f.probe.deadline)
            self.assertEqual(value['terminal_actions'],[])
            self.assertEqual(value['terminal'],f.terminal)
            self.assertEqual(value['action_ids'],[(40,f.seal)])

    def test_same_snapshot_connects_original_actions_registration_and_stop_result_row(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));self.database(f)
            stack.enter_context(patch.object(sync,'checked_source_records',return_value=f.cut['sources']))
            value=f.probe._snapshot(f.probe.deadline)
            self.assertEqual(value['action_ids'],f.cut['action_ids'])
            self.assertEqual(value['global_cleanups'],[f.cleanup])
            SyncDestroyFactsTests().facts(f,stack,exited=True)
            self.assertEqual(f.probe._cut_facts(value)['original_end']['stop']['result'],52)

    def test_original_sql_snapshot_does_not_mix_a_concurrent_owner_write(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));database=self.database(f);writes=[]
            def records(*_):
                if not writes:
                    changed=dict(f.cut['native'],owner=2)
                    with sqlite3.connect(database) as other:
                        other.execute('UPDATE sandboxes SET state_blob=?',(json.dumps(changed).encode(),))
                    writes.append(True)
                return f.cut['sources']
            stack.enter_context(patch.object(sync,'checked_source_records',side_effect=records))
            self.assertEqual(f.probe._snapshot(f.probe.deadline)['native']['owner'],1)
            with self.assertRaisesRegex(ValueError,'owner changed'):f.probe._snapshot(f.probe.deadline)

    def test_wrong_native_logical_registration_is_a_rejection_not_an_import_failure(self):
        for fault in ('wrong-command','missing','noncanonical'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));database=self.database(f)
                with sqlite3.connect(database) as conn:
                    if fault=='wrong-command':conn.execute('UPDATE controller_effect_keys SET logical_json=? WHERE id=502',(b'{"command":{"StopVm":{"sandbox":4,"generation":2,"vm":10,"epoch":4}}}',))
                    if fault=='missing':conn.execute('DELETE FROM controller_effect_keys WHERE id=502')
                    if fault=='noncanonical':
                        action=copy.deepcopy(f.cleanup);action['identity']['logical_action']='effect-0503'
                        conn.execute('UPDATE outbox SET action_json=? WHERE id=53',(json.dumps(action).encode(),))
                stack.enter_context(patch.object(sync,'checked_source_records',return_value=f.cut['sources']))
                with self.assertRaises(ValueError):f.probe._snapshot(f.probe.deadline)

    def test_full_destroy_or_original_entry_mismatch_is_rejected(self):
        for path,value in [(['entries',1,'intent','center','center'],'other-center'),
                           (['entries',1,'intent','retention','window'],8),
                           (['entries',1,'intent','request','binding','expected','version'],3),
                           (['entries',1,'intent','request','binding','node','node_generation'],2),
                           (['entries',0,'intent','request','operation','capture'],6)]:
            with self.subTest(path=path),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));database=self.database(f)
                with sqlite3.connect(database) as conn:
                    raw,=conn.execute('SELECT state_json FROM controller_lifecycle_inbox').fetchone()
                    value_inbox=json.loads(raw);change(value_inbox['sessions'][0],path,value)
                    encoded=json.dumps(value_inbox).encode()
                    conn.execute('UPDATE controller_lifecycle_inbox SET state_json=?,sha256=?',(encoded,hashlib.sha256(encoded).digest()))
                stack.enter_context(patch.object(sync,'checked_source_records',return_value=f.cut['sources']))
                with self.assertRaises(ValueError):f.probe._snapshot(f.probe.deadline)


class SyncDestroyRecoveryTests(unittest.TestCase):
    def recovery(self, f, stack, fault='none'):
        p=f.probe;p.terminal_accepted=None
        states=[f.held,f.held,f.cut]
        p._snapshot=Mock(side_effect=states)
        calls=[]
        def exited(descriptor):return bool(calls)  # Both exact pidfds remain live until this command.
        def publish(*args):
            calls.append(copy.deepcopy(args[2]))
            if fault=='publish-unknown':raise OSError('unknown original publication')
        stack.enter_context(patch.object(sync,'pidfd_exited',side_effect=exited))
        stack.enter_context(patch.object(sync,'original_process',return_value=True))
        stack.enter_context(patch.object(sync,'original_json',return_value=dict(epoch=4,terminal=True)))
        stack.enter_context(patch.object(sync,'checked_file',return_value=p.slot_gate['file']))
        sent=stack.enter_context(patch.object(sync,'gate_publish',side_effect=publish))
        def file(name):
            if name=='command.json' and fault=='save-command':raise OSError('cannot confirm exact published bytes')
            return (dict(calls[0],command='release') if fault=='consumed' else calls[0], None)
        p._save_file=Mock(side_effect=file)
        if fault=='exit-code':p.host.wait.return_value=1
        if fault=='bad-audit':f.audit['operations'][0]['local_operation']=51
        peer=Mock(return_value={'original_peer':'progress'})
        if fault=='peer':peer.side_effect=TimeoutError('original peer unknown')
        if fault=='expired':stack.enter_context(patch.object(sync,'remaining',side_effect=TimeoutError('original deadline expired')))
        return sent,peer,calls

    def test_both_sync_cases_use_exact_fail_then_original_end_audit_and_new_manager(self):
        for stage in capture.SYNC_PHASES:
            with self.subTest(stage=stage),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory),stage);sent,peer,calls=self.recovery(f,stack)
                value=f.probe.recover_after_destroy(f.client.current_acceptance,f.probe.deadline,observe_peer=peer)
                self.assertTrue(value['original_recovery_resumed']);self.assertTrue(f.probe.recovered)
                sent.assert_called_once();peer.assert_called_once();self.assertEqual(calls[0]['command'],'fail')
                self.assertEqual(calls[0]['full_work_binding'],[9]*32)
                f.runner.services.processes.start.assert_called_once_with('host-0')
                self.assertFalse(value['public_completion'])

    def test_peer_or_expired_cut_keeps_original_gate_without_sending_failure_command(self):
        for fault in ('peer','expired'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));sent,peer,_=self.recovery(f,stack,fault)
                with self.assertRaises(TimeoutError):f.probe.recover_after_destroy(f.client.current_acceptance,f.probe.deadline,observe_peer=peer)
                sent.assert_not_called();self.assertFalse(f.probe.recovered)
                self.assertFalse(f.probe.terminal_failure_state()['command_publish_attempted'])
                f.runner.services.processes.start.assert_not_called()

    def test_uncertain_publication_or_post_publish_failure_never_retries_or_reports_no_command(self):
        for fault in ('publish-unknown','save-command','consumed','exit-code','bad-audit'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));sent,peer,_=self.recovery(f,stack,fault)
                with self.assertRaises((OSError,ValueError)):
                    f.probe.recover_after_destroy(f.client.current_acceptance,f.probe.deadline,observe_peer=peer)
                state=f.probe.terminal_failure_state()
                self.assertTrue(state['command_publish_attempted']);self.assertFalse(f.probe.recovered)
                self.assertEqual(state['command_sent'],fault!='publish-unknown')
                with self.assertRaises(ValueError):
                    f.probe.recover_after_destroy(f.client.current_acceptance,f.probe.deadline,observe_peer=peer)
                self.assertEqual(sent.call_count,1);f.runner.services.processes.start.assert_not_called()


class SyncDestroySceneTests(unittest.TestCase):
    def exercise(self,stage,*,failure=False):
        events=[];clients={};lifetime=dict(started_at_ms=1,end_at_ms=2,max_end_at_ms=3,startup_deadline_ms=2)
        for owner in ('1','2'):
            c=Mock(spec=scene.PublicUnloadScenario);c.owner=owner;c.session='session-'+owner
            c.current_acceptance=dict(key={},accepted_version=1);c.unload_acceptance=dict(key={},accepted_version=2)
            c.concurrent_unload_replies=[c.unload_acceptance]*2;c.resume_request={};c.witness={}
            c.timings=[dict(sent_monotonic=time.monotonic())];c.timing_report.return_value=[]
            c.last_http_status=None;c.failure_step='controlled'
            c.wait_running.return_value=dict(phase='running');c.wait_unloaded.return_value=dict(phase='unloaded')
            c.query.return_value=dict(phase='unloading' if owner=='1' else 'running')
            c.assert_terminal_unload_replay.side_effect=lambda *,phase:dict(phase=phase)
            for name in ('create','unload','resume','destroy','wait_cleaned','run_public_workload','assert_resumed_workload'):
                getattr(c,name).side_effect=lambda *_,n=name,o=owner,**__:events.append(n+'-'+o) or {}
            clients[owner]=c
        peer=clients['2'];peer.unload_acceptance=None
        peer.current_acceptance=dict(session=peer.session,key=dict(deployment='original',owner='2',window=1,sequence=1),accepted_version=1)
        peer.wait_running.return_value=dict(session=peer.session,operation=peer.current_acceptance['key'],current_version=1,
                                           phase='running',unload_outcome=None,create_refusal=None)
        peer.query.return_value=peer.wait_running.return_value
        state={}
        def initialize(*_):
            events.append('run_public_workload-2');state.update(nonce='a'*64,activation_nonce='b'*64,
                metadata_generation=1,counter=1,pid=41,start_time_ticks=7)
            peer.witness=copy.deepcopy(state);peer.data_token='original-peer-token';return {}
        peer.run_public_workload.side_effect=initialize
        sdk=Mock();sdk.files.read.side_effect=lambda _: 'disk-counter:'+str(state['counter']);peer.sdk.return_value=sdk
        def witness(_,command):
            if command=='advance':state['counter']+=1
            return copy.deepcopy(state)
        peer.witness_command.side_effect=witness
        for name in ('check_view','_advance_same_running_workload'):
            setattr(peer,name,getattr(scene.PublicUnloadScenario,name).__get__(peer))
        real_running=scene.PublicUnloadScenario.assert_running_workload.__get__(peer)
        def progress(*args):events.append('real-peer-progress');return real_running(*args)
        peer.assert_running_workload.side_effect=progress
        calls=0
        def running(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context=dict(position=0 if session=='session-2' or calls==1 else 1))
        probe=Mock(spec=sync.SyncDestroyProbe)
        for name in ('arm','after_acceptance','cleanup','close'):
            getattr(probe,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        def recovery(*_,observe_peer):
            events.append('recover');observe_peer()
            if failure:raise ValueError('unknown original terminal End')
            events.append('original-cut-recovered');return {}
        probe.recover_after_destroy.side_effect=recovery
        probe.terminal_failure_state.return_value=dict(command_publish_attempted=True,command_sent=False,public_completion=False)
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            runner=NS(root=Path(directory),owner_key='owner',foreign_key='other',quota=dict(deployment_control=directory),
                services=NS(sdk_url='http://127.0.0.1:1'),measurement_identity=lambda:{},start=lambda _:None,
                create_parameters=lambda **_:{},record_running=Mock(side_effect=running),
                unload_evidence=Mock(return_value=dict(facts=lifetime,original_position=dict(position=0),original_capture_responsibilities=1)),
                restart_managers=Mock(return_value=dict(original_unload_operation={})),confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}),settlement_evidence=Mock(return_value={}),
                finish=Mock(),fail_resume_gate=Mock(return_value='not-armed'),close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            stack.enter_context(patch.object(scene,'PublicUnloadScenario',side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            factory=stack.enter_context(patch.object(sync,'SyncDestroyProbe',return_value=probe))
            args=NS(scenario='capture-'+stage+'-destroy',scenario_timeout=60,activation_gate_timeout_ms=60000)
            if failure:
                with self.assertRaisesRegex(ValueError,'unknown original terminal End'):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete']);self.assertIn('sync_destroy_failure_state',report)
                peer.destroy.assert_not_called();runner.settlement_evidence.assert_not_called();probe.cleanup.assert_not_called()
                clients['1'].wait_cleaned.assert_not_called()
            else:
                report=scene.exercise(args);self.assertTrue(report['complete']);self.assertEqual(len(report['cycles']),1)
                self.assertTrue(report['cycles'][0]['sync_held_unload_destroyed'])
                self.assertEqual([v['phase'] for v in report['terminal_unload_replays']],['terminating','cleaned'])
                self.assertEqual(events.count('real-peer-progress'),2);self.assertEqual(state['counter'],3)
                self.assertLess(events.index('run_public_workload-2'),events.index('arm'))
                self.assertLess(events.index('original-cut-recovered'),events.index('destroy-2'))
                peer.destroy.assert_called_once();probe.cleanup.assert_called_once();runner.settlement_evidence.assert_called_once()
            self.assertEqual(clients['1'].resume.call_count,1);self.assertEqual(clients['1'].unload.call_count,2)
            clients['1'].wait_unloaded.assert_called_once();clients['1'].destroy.assert_called_once()
            probe.finish.assert_not_called();peer.assert_continued_workload.assert_not_called()
            factory.assert_called_once();self.assertEqual(factory.call_args.args[3],stage)
            probe.close.assert_called_once();runner.finish.assert_called_once()

    def test_both_original_scenes_use_real_normal_peer_witness_and_original_terminal_cleanup(self):
        for stage in capture.SYNC_PHASES:self.exercise(stage)

    def test_unknown_end_keeps_two_acceptances_without_reunload_second_resume_or_peer_destroy(self):
        for stage in capture.SYNC_PHASES:self.exercise(stage,failure=True)


if __name__=='__main__':
    unittest.main()
