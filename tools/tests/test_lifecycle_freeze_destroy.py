"""E8 consumers with original SQL/framing and controlled providers, never KVM."""
import copy
from contextlib import ExitStack, closing, contextmanager
import hashlib
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import signal
import struct
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
                    logical_action='effect-'+str({'FreezeVm':1,'InstallFence':2,'StopVm':3,'CleanupVm':4}[kind]),
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
    native=dict(owner=1,terminal=True,phase='Revoking',generation=2,current_vm=9,net_slot=10,epoch=4,
        current_operation=dict(kind='Kill',id=5,epoch=4,status='Pending'))
    kill_ledger=dict(sandbox=4,operation=copy.deepcopy(native['current_operation']),
        input={'Kill': {'generation': 2}},result=None)
    snapshot=dict(original=old,terminal=terminal,local=local,native=native,sources=rows,original_actions=[(freeze,'dispatched',None)],
        terminal_actions=[(fence,'receipted',receipt(fence,'Fence',dict(epoch=4,terminal=True))),(stop,'dispatched',None)],
        global_pending_stops=[stop],lifetime=lifetime,logical_action_registered=True,native_kill_ledger=kill_ledger)
    cleanup_snapshot=copy.deepcopy(snapshot)
    cleanup_snapshot['terminal_actions'][1]=(stop,'receipted',receipt(stop,'Stopped',dict(pid=123,start_ticks=456)))
    cleanup_snapshot['terminal_actions'].append((cleanup,'dispatched',None));cleanup_snapshot['global_pending_stops']=[]
    cleanup_snapshot['logical_action_registered']=False
    settings=dict(namespace=namespace,database=str(root/'state.db'),node_state_dir=str(root/'state'),chroot_base=str(root/'jails'))
    config=dict(inbox=dict(node=node),native=dict(maintenance=dict(capacity=dict(max_sql_rows=64,max_sql_value_bytes=65536))))
    runner=NS(services=NS(native_pools=[settings],host_configs=[config],processes=NS(bindings={'host-0':dict(uid=os.getuid(),gid=os.getgid())},children={})),
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
    runner.services.processes.children['host-0']=p.host
    inbox=dict(sessions=[dict(id=client.session,current_version=5,terminal=True,local=local,entries=[old,terminal])])
    return NS(probe=p,runner=runner,node=node,inbox=inbox,snapshot=snapshot,cleanup=cleanup_snapshot,accepted=accepted,
        unload=unload,destroy=destroy,freeze=freeze,stop=stop,receipt=receipt,rows=rows)


def freeze_window_journal(f):
    p = f.probe
    path = Path(f.runner.services.native_pools[0]['database'])
    row = dict(f.inbox['sessions'][0], current_version=4, terminal=False, entries=[p.unload_entry])
    raw = json.dumps({'sessions': [row]}).encode()
    with closing(sqlite3.connect(path)) as conn, conn:
        conn.execute('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER)')
        conn.execute('CREATE TABLE outbox(id INTEGER PRIMARY KEY,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB)')
        conn.execute('CREATE TABLE controller_effect_keys(id INTEGER PRIMARY KEY,logical_json BLOB NOT NULL UNIQUE)')
        conn.execute('CREATE TABLE sandboxes(sandbox_id TEXT PRIMARY KEY,owner_id TEXT,state_blob BLOB)')
        conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',
                     (raw, hashlib.sha256(raw).digest(), 65536))
        conn.execute('INSERT INTO outbox VALUES(1,?,?,?,?,NULL)',
                     (p.config['sandbox'], f.freeze['identity']['operation'],
                      json.dumps(f.freeze).encode(), 'dispatched'))
        conn.execute('INSERT INTO controller_effect_keys VALUES(1,?)',
                     (json.dumps(f.freeze['command']['logical']).encode(),))
        conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',
                     (p.config['sandbox'], f.freeze['identity']['owner'], json.dumps(pending_pause(f)).encode()))
    path.chmod(0o600)
    return path


def pending_pause(f):
    return dict(f.snapshot['native'], owner=1, terminal=False, phase='Pausing', epoch=3,
                current_operation=dict(id=4, epoch=3, kind='Pause', status='Pending'))


def ordered_kill_journal(f):
    path = freeze_window_journal(f)
    raw = json.dumps(f.inbox).encode()
    fence = f.snapshot['terminal_actions'][0][0]
    ledger = f.snapshot['native_kill_ledger']
    with closing(sqlite3.connect(path)) as conn, conn:
        conn.execute('CREATE TABLE operations(operation_id TEXT PRIMARY KEY,sandbox_id TEXT NOT NULL,'
                     'owner_id TEXT NOT NULL,state_blob BLOB NOT NULL)')
        conn.execute('CREATE TABLE sandbox_lifetimes(sandbox_id TEXT PRIMARY KEY,owner_id TEXT,'
                     'started_at_ms INTEGER,deadline_ms INTEGER,max_deadline_ms INTEGER,startup_deadline_ms INTEGER)')
        conn.execute('UPDATE controller_lifecycle_inbox SET state_json=?,sha256=?',
                     (raw, hashlib.sha256(raw).digest()))
        conn.execute('UPDATE sandboxes SET state_blob=?', (json.dumps(f.snapshot['native']).encode(),))
        conn.execute('INSERT INTO outbox VALUES(2,?,?,?,?,NULL)',
                     (f.probe.config['sandbox'], fence['identity']['operation'], json.dumps(fence).encode(), 'ready'))
        conn.execute('INSERT INTO operations VALUES(?,?,?,?)',
                     (fence['identity']['operation'], f.probe.config['sandbox'],
                      fence['identity']['owner'], json.dumps(ledger).encode()))
        conn.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?)',
                     (f.probe.config['sandbox'], fence['identity']['owner'], 900, 1000, 1100, 1000))
    return path


class FrozenFactsTests(unittest.TestCase):
    def test_terminal_history_rejects_numeric_aliases_without_turning_them_into_pending(self):
        for fault in ('none','version','sandbox','owner-float','owner-bool','pending','pending-float'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;p._bind_destroy(f.accepted(f.destroy))
                inbox=copy.deepcopy(f.inbox);row=inbox['sessions'][0]
                if fault=='version':row['current_version']=5.0
                if fault=='sandbox':row['local']['sandbox']=4.0
                if fault=='owner-float':row['local']['owner']=1.0
                if fault=='owner-bool':row['local']['owner']=True
                if fault.startswith('pending'):
                    row['current_version']=4.0 if fault=='pending-float' else 4
                    row['terminal']=False
                if fault=='none':self.assertEqual(p._terminal_entries(inbox)[1]['intent'],f.destroy)
                elif fault=='pending':
                    with self.assertRaises(capture._AwaitTerminal):p._terminal_entries(inbox)
                else:
                    with self.assertRaises(ValueError):p._terminal_entries(inbox)
                self.assertFalse(p.recovered);self.assertFalse(p.stop_released);self.assertFalse(p.continue_sent)

    def test_terminal_actions_from_original_sql_keep_protocol_generation_and_attempt_types(self):
        cases=(None,('protocol_version',True),('protocol_version',1.0),
               ('generation',2.0),('attempt',False),('attempt',0.0))
        for case in cases:
            with self.subTest(case=case),tempfile.TemporaryDirectory() as directory,sqlite3.connect(':memory:') as conn:
                f=fixture(Path(directory));action=copy.deepcopy(f.stop)
                if case is not None:action['identity'][case[0]]=case[1]
                conn.execute('CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB)')
                conn.execute('INSERT INTO outbox VALUES(1,?,?,?,\'dispatched\',NULL)',
                    (f.stop['identity']['sandbox'],f.stop['identity']['operation'],json.dumps(action).encode()))
                capacity=f.runner.services.host_configs[0]['native']['maintenance']['capacity']
                if case is None:
                    self.assertEqual(f.probe._terminal_outbox(conn,f.probe.prior['local'],5,capacity),[(action,'dispatched',None)])
                else:
                    with self.assertRaises(ValueError):f.probe._terminal_outbox(conn,f.probe.prior['local'],5,capacity)

    def test_freeze_receipts_keep_complete_identity_and_command_json_types(self):
        identity_cases=(None,('protocol_version',True),('protocol_version',1.0),('epoch',3.0),
                        ('generation',2.0),('attempt',False),('attempt',0.0))
        for status in ('observed_success','unknown'):
            for case in identity_cases:
                with self.subTest(status=status,identity=case),tempfile.TemporaryDirectory() as directory:
                    f=fixture(Path(directory));body=dict(pid=123,start_ticks=456,boot_id='boot')
                    receipt=f.receipt(f.freeze,'Frozen',body);receipt['outcome']['status']=status
                    if case is not None:receipt['identity'][case[0]]=case[1]
                    row=(f.freeze,'receipted',receipt)
                    if case is None:
                        self.assertEqual(f.probe._success_observation(row,'Frozen'),body if status=='observed_success' else None)
                    else:
                        with self.assertRaises(ValueError):f.probe._success_observation(row,'Frozen')
        for field,value in (('sandbox',4.0),('generation',2.0),('vm',9.0),('epoch',3.0)):
            with self.subTest(command_field=field),tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));receipt=f.receipt(f.freeze,'Frozen',dict(pid=123,start_ticks=456,boot_id='boot'))
                receipt['outcome']['details']['command']['FreezeVm'][field]=value
                with self.assertRaises(ValueError):f.probe._success_observation((f.freeze,'receipted',receipt),'Frozen')

    def test_original_stop_barrier_rejects_numeric_aliases_before_releasing_any_effect(self):
        paths=(None,('native','generation'),('native','current_vm'),('native','net_slot'),
               ('native','epoch'),('native','current_operation','id'),('native','current_operation','epoch'),
               ('terminal_actions',0,0,'identity','epoch'),('terminal_actions',1,0,'identity','epoch'))
        for probe_type in (frozen.FreezeDestroyProbe, frozen.OrderedFreezeDestroyProbe):
            for path in paths:
                with self.subTest(probe=probe_type.__name__,path=path),tempfile.TemporaryDirectory() as directory:
                    f=fixture(Path(directory));p=f.probe;p.__class__=probe_type;s=copy.deepcopy(f.snapshot)
                    if path is not None:
                        value=s
                        for key in path[:-1]:value=value[key]
                        value[path[-1]]=float(value[path[-1]])
                    with patch.object(frozen,'original_json',return_value=dict(epoch=4,terminal=True)), \
                            patch.object(frozen.signal,'pidfd_send_signal',create=True) as sent:
                        if path is None:self.assertEqual(p._stop_barrier(s),dict(epoch=4,terminal=True))
                        else:
                            with self.assertRaises(ValueError):p._stop_barrier(s)
                        sent.assert_not_called();self.assertFalse(p.stop_released);self.assertFalse(p.recovered)

    def test_cleanup_epoch_alias_cannot_certify_exit_or_reopen_the_host(self):
        for probe_type in (frozen.FreezeDestroyProbe, frozen.OrderedFreezeDestroyProbe):
            for value in (4,4.0):
                with self.subTest(probe=probe_type.__name__,epoch_type=type(value).__name__),tempfile.TemporaryDirectory() as directory:
                    f=fixture(Path(directory));p=f.probe;p.__class__=probe_type
                    p.recovered=p.stop_released=True;p.continue_sent=p.continue_source_before_stop
                    p.before_stop=copy.deepcopy(f.snapshot);snapshot=copy.deepcopy(f.cleanup)
                    snapshot['terminal_actions'][2][0]['identity']['epoch']=value
                    p.terminal_host=Mock();p.terminal_host.wait.return_value=86;p._read_terminal=Mock(return_value=snapshot)
                    p.runtime.services.processes.start=Mock();p.runtime.services.wait_ready=Mock()
                    with patch.object(frozen,'pidfd_exited',return_value=True), \
                            patch.object(frozen,'original_process',return_value=False):
                        if type(value) is int:
                            self.assertTrue(p.assert_terminal_before_gc(p.deadline)['original_source_exited'])
                            p.runtime.services.processes.start.assert_called_once_with('host-0')
                        else:
                            with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                            self.assertIsNone(p.terminal_cut);p.runtime.services.processes.start.assert_not_called()

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

    def test_destroy_window_rechecks_original_freeze_in_read_only_sql(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            path = freeze_window_journal(f)
            original = json.dumps(f.freeze).encode()
            operation = f.freeze['identity']['operation']
            replacement = copy.deepcopy(f.freeze)
            replacement['identity']['logical_action'] = 'effect-17'
            with patch.object(capture, 'checked_source_records', return_value=f.rows), \
                    patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                first = p.observe_destroy_window(p.deadline)
                self.assertEqual(first['freeze_identity'], f.freeze['identity'])
                self.assertEqual(first['unload_accepted_version'], 4)
                self.assertEqual(first['host_exit_code'], 86)
                self.assertIs(first['freeze_sql_receipt_pending'], True)
                self.assertEqual(p.observe_destroy_window(p.deadline), first)
                for action, state, receipt, message in (
                        (original, 'dispatched', b'{}', 'no SQL receipt'),
                        (original, 'dispatched', b'null', 'stored outbox receipt cannot be JSON null'),
                        (original, 'receipted', None, 'no SQL receipt'),
                        (json.dumps(replacement).encode(), 'dispatched', None, 'complete Freeze changed')):
                    with self.subTest(state=state, receipt=receipt, message=message):
                        with closing(sqlite3.connect(path)) as conn, conn:
                            conn.execute('UPDATE outbox SET action_json=?, dispatch_state=?, receipt_json=?',
                                         (action, state, receipt))
                        with self.assertRaisesRegex(ValueError, message):
                            p.observe_destroy_window(p.deadline)
                with closing(sqlite3.connect(path)) as conn, conn:
                    conn.execute('UPDATE outbox SET action_json=?, dispatch_state=?, receipt_json=NULL',
                                 (original, 'dispatched'))
                self.assertEqual(held.call_count, 2)
                namespace = f.runner.services.native_pools[0]['namespace']
                other_operation = f'op-{namespace}-4-2'
                with closing(sqlite3.connect(path)) as conn:
                    for kind in ('SealSnapshot', 'CommitSnapshot'):
                        action = copy.deepcopy(f.freeze)
                        action['identity'].update(logical_action='effect-17',
                            resource=dict(kind='snapshot', id=f'snapshot-{namespace}-7'))
                        fields = dict(sandbox=4, epoch=3, generation=2, snapshot=7,
                                      manifest=dict(digest=5))
                        if kind == 'SealSnapshot':
                            fields.update(vm=9, workspace=6)
                        action['command']['logical']['command'] = {kind: fields}
                        unknown = json.dumps(dict(identity=action['identity'], outcome=dict(
                            status='unknown', details=dict(reason='observation_unavailable',
                                                           last_observation=None)))).encode()
                        for state, receipt in (('ready', None), ('dispatched', None), ('receipted', unknown)):
                            with self.subTest(kind=kind, state=state):
                                with conn:
                                    conn.execute('INSERT OR REPLACE INTO outbox VALUES(2,?,?,?,?,?)',
                                        (p.config['sandbox'], operation, json.dumps(action).encode(), state, receipt))
                                with self.assertRaisesRegex(ValueError, 'outbox contains a capture action'):
                                    p.observe_destroy_window(p.deadline)
                        action['identity']['operation'] = other_operation
                        with conn:
                            conn.execute('INSERT OR REPLACE INTO outbox VALUES(2,?,?,?,?,NULL)',
                                         (p.config['sandbox'], other_operation, json.dumps(action).encode(), 'ready'))
                        with self.subTest(kind=kind, operation=other_operation):
                            self.assertEqual(p.observe_destroy_window(p.deadline), first)
                    with conn:
                        conn.execute('DELETE FROM outbox WHERE id=2')
                self.assertEqual(held.call_count, 4)
                self.assertIs(first['capture_outbox_actions_absent'], True)
                f.runner.services.processes.children['host-0'] = Mock(pid=p.host.pid)
                with self.assertRaisesRegex(ValueError, 'original Freeze cut Host'):
                    p.observe_destroy_window(p.deadline)

    def test_destroy_window_reads_freeze_registration_in_the_same_sql_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            path = freeze_window_journal(f)
            logical = f.freeze['command']['logical']
            capacity = f.runner.services.host_configs[0]['native']['maintenance']['capacity']
            with closing(sqlite3.connect(path)) as writer, patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                self.assertEqual(writer.execute('PRAGMA journal_mode=WAL').fetchone(), ('wal',))

                def replace_after_outbox_read(reader, _config, _deadline):
                    self.assertTrue(reader.in_transaction)
                    variant = copy.deepcopy(logical)
                    variant['command']['FreezeVm']['vm'] += 1
                    with writer:
                        writer.execute('UPDATE controller_effect_keys SET logical_json=? WHERE id=1',
                                       (json.dumps(variant).encode(),))
                    return f.rows

                with patch.object(capture, 'checked_source_records', side_effect=replace_after_outbox_read):
                    before = p.observe_destroy_window(p.deadline)
                held.reset_mock()
                with patch.object(capture, 'checked_source_records', return_value=f.rows):
                    with self.assertRaisesRegex(ValueError, 'Freeze logical registration differs'):
                        p.observe_destroy_window(p.deadline)
                    self.assertIs(before['freeze_logical_action_registered'], True)
                    type_alias = copy.deepcopy(logical)
                    type_alias['command']['FreezeVm']['vm'] = float(type_alias['command']['FreezeVm']['vm'])
                    for case, registration in (
                            ('different JSON type', json.dumps(type_alias).encode()),
                            ('over bound', b'x' * (capacity['max_sql_value_bytes'] + 1)),
                            ('missing', None)):
                        with self.subTest(case=case):
                            with writer:
                                writer.execute('DELETE FROM controller_effect_keys WHERE id=1')
                                if registration is not None:
                                    writer.execute('INSERT INTO controller_effect_keys VALUES(1,?)', (registration,))
                            with self.assertRaisesRegex(ValueError, 'logical'):
                                p.observe_destroy_window(p.deadline)
                    held.assert_not_called()
                    with writer:
                        writer.execute('INSERT INTO controller_effect_keys VALUES(1,?)',
                                       (json.dumps(logical).encode(),))
                    self.assertIs(p.observe_destroy_window(p.deadline)['freeze_logical_action_registered'], True)
                    held.assert_called_once()

    def test_destroy_window_keeps_original_backing_and_material_responsibility(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            freeze_window_journal(f)
            with patch.object(capture, 'checked_source_records', return_value=f.rows) as records, \
                    patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                before = p.observe_destroy_window(p.deadline)
                held.assert_called_once()

                for change in ('missing backing', 'closed pin', 'changed charge', 'changed work'):
                    with self.subTest(change=change):
                        rows = copy.deepcopy(f.rows)
                        if change == 'missing backing':
                            rows.pop(0)
                        elif change == 'closed pin':
                            rows[0]['restore']['material_use'] = False
                        elif change == 'changed charge':
                            rows[0]['capture']['full']['disk_bytes'] += 1
                            rows[0]['capture']['reserved_material_bytes'] += 2
                        else:
                            rows[0]['work']['anchor']['local']['operation'] += 1
                        records.return_value = rows
                        with self.assertRaisesRegex(ValueError, 'material responsibility changed at public Destroy'):
                            p.observe_destroy_window(p.deadline)
                held.assert_called_once()

                rows = copy.deepcopy(f.rows)
                rows[0]['observation'] = {'sample': 2}
                unrelated = copy.deepcopy(rows[1])
                unrelated['work']['anchor']['local']['sandbox'] += 1
                unrelated['work']['anchor']['request']['request']['binding']['session'] = 'sandbox-2'
                records.return_value = [unrelated, *rows]
                self.assertEqual(p.observe_destroy_window(p.deadline), before)
                self.assertEqual(held.call_count, 2)
                self.assertIs(before['original_pin_and_positive_charge_retained'], True)

    def test_destroy_window_requires_original_nonterminal_host_inbox(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            path = freeze_window_journal(f)
            original = dict(f.inbox['sessions'][0], current_version=4, terminal=False,
                            entries=[p.unload_entry])
            with closing(sqlite3.connect(path)) as writer, \
                    patch.object(capture, 'checked_source_records', return_value=f.rows), \
                    patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                before = p.observe_destroy_window(p.deadline)
                for variant in (
                        dict(original, terminal=True),
                        dict(original, terminal=0),
                        dict(original, entries=f.inbox['sessions'][0]['entries'])):
                    with self.subTest(terminal=variant['terminal'], entries=len(variant['entries'])):
                        raw = json.dumps({'sessions': [variant]}).encode()
                        with writer:
                            writer.execute('UPDATE controller_lifecycle_inbox SET state_json=?, sha256=?',
                                           (raw, hashlib.sha256(raw).digest()))
                        with self.assertRaisesRegex(ValueError, 'Host inbox must remain nonterminal'):
                            p.observe_destroy_window(p.deadline)
                held.assert_called_once()
                self.assertEqual(before['unload_accepted_version'], 4)

    def test_destroy_window_rejects_a_different_native_pause(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            path = freeze_window_journal(f)
            pause = pending_pause(f)
            with closing(sqlite3.connect(path)) as writer, \
                    patch.object(capture, 'checked_source_records', return_value=f.rows), \
                    patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                before = p.observe_destroy_window(p.deadline)
                for field, value in (
                        ('terminal', True), ('phase', 'Paused'), ('owner', 1.0), ('epoch', 4),
                        ('generation', 2.0), ('current_vm', 10), ('net_slot', 11),
                        ('current_operation', None),
                        ('current_operation', dict(pause['current_operation'], id=5, kind='Kill')),
                        ('current_operation', dict(pause['current_operation'], status='Succeeded'))):
                    with self.subTest(field=field, value=value):
                        with writer:
                            writer.execute('UPDATE sandboxes SET state_blob=?',
                                           (json.dumps(dict(pause, **{field: value})).encode(),))
                        with self.assertRaisesRegex(ValueError, 'native Pause must remain current'):
                            p.observe_destroy_window(p.deadline)
                held.assert_called_once()
                self.assertEqual(before['host_nonterminal_pause'], pause['current_operation'])

    def test_destroy_window_reads_native_pause_in_the_freeze_sql_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            path = freeze_window_journal(f)
            with closing(sqlite3.connect(path)) as writer, patch.object(p, '_check_source_hold') as held, \
                    patch.object(p, '_observe_destroy_endpoint', return_value={}):
                self.assertEqual(writer.execute('PRAGMA journal_mode=WAL').fetchone(), ('wal',))

                def record_terminal_on_another_connection(reader, _config, _deadline):
                    self.assertTrue(reader.in_transaction)
                    with writer:
                        writer.execute('UPDATE sandboxes SET state_blob=?',
                                       (json.dumps(dict(f.snapshot['native'], owner=1)).encode(),))
                    return f.rows

                with patch.object(capture, 'checked_source_records',
                                  side_effect=record_terminal_on_another_connection):
                    before = p.observe_destroy_window(p.deadline)
                with patch.object(capture, 'checked_source_records', return_value=f.rows):
                    with self.assertRaisesRegex(ValueError, 'native Pause must remain current'):
                        p.observe_destroy_window(p.deadline)
                held.assert_called_once()
                self.assertEqual(before['host_nonterminal_pause'], pending_pause(f)['current_operation'])

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

    def test_original_never_released_end_is_allowed_without_inventing_an_api_no_request(self):
        for fault in ('none','initial-end','binding','seal','worker','completed','material','returned','full-changed'):
            with tempfile.TemporaryDirectory() as directory:
                f=fixture(Path(directory));p=f.probe;s=copy.deepcopy(f.snapshot)
                full=dict(pause=dict(binding=dict(operation=f.unload['request']['binding'],source_incarnation=7,capture=5),
                    source_work_binding=[1]*32,node=f.node,source_resource=f.freeze['identity']['resource'],
                    source={k:p.prior['source'][k] for k in ('pid','start_time_ticks','boot_id')}),
                    compatibility={'original':'saved profile'},state_bytes=1,memory_bytes=2,disk_bytes=3,api_timeout_ms=1000,worker_timeout_ms=1000)
                captured=dict(full=full,operation=4,reserved_material_bytes=589836,action=None,worker=None,completed=None,material_bytes=None,end=None)
                p.initial_sources[1]['capture']=copy.deepcopy(captured)
                s['sources'][1]['capture']=copy.deepcopy(captured);c=s['sources'][1]['capture']
                digest=list(hashlib.sha256(json.dumps(full,ensure_ascii=False,separators=(',',':')).encode()).digest())
                c['end']=dict(state='never_released',full_work_binding=digest,seal_action=None)
                if fault=='binding':c['end']['full_work_binding'][0]^=1
                if fault=='seal':c['end']['seal_action']=f.freeze
                if fault=='worker':c['worker']={'unknown':'worker'}
                if fault=='completed':c['completed']=[7]*32
                if fault=='material':c['material_bytes']=6
                if fault=='returned':c['end']={'state':'worker_ended','observation':{'outcome':'no_request'}}
                if fault=='full-changed':c['full']['memory_bytes']+=1
                with patch.object(frozen,'original_json',return_value=dict(epoch=4,terminal=True)),self.subTest(fault=fault):
                    if fault=='none':
                        self.assertTrue(p._stop_barrier(s)['terminal'])
                        self.assertEqual(s['sources'][1]['capture']['end']['state'],'never_released')
                    elif fault=='initial-end':
                        with self.assertRaises(ValueError):p._no_full(s['sources'])
                    else:
                        with self.assertRaises(ValueError):p._stop_barrier(s)

    def test_cleanup_needs_original_stop_receipt_kernel_exit_and_pending_cleanup(self):
        for probe_type in (frozen.FreezeDestroyProbe, frozen.OrderedFreezeDestroyProbe):
            for fault in ('none','pid','start-ticks','stop-pending','stop-attempt','stop-logical-action',
                          'no-exit','still-live','cleanup-saved','cleanup-epoch','freeze-missing','extra-full'):
                with self.subTest(probe=probe_type.__name__,fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                    f=fixture(Path(directory));p=f.probe;p.__class__=probe_type
                    p.recovered=p.stop_released=True;p.continue_sent=p.continue_source_before_stop
                    p.before_stop=copy.deepcopy(f.snapshot)
                    s=copy.deepcopy(f.cleanup);p.terminal_host=Mock();p.terminal_host.wait.return_value=86
                    p._read_terminal=Mock(return_value=s);p.runtime.services.processes.start=Mock();p.runtime.services.wait_ready=Mock()
                    if fault=='pid':s['terminal_actions'][1][2]['outcome']['details']['result']['Stopped']['pid']+=1
                    if fault=='start-ticks':s['terminal_actions'][1][2]['outcome']['details']['result']['Stopped']['start_ticks']+=1
                    if fault=='stop-pending':
                        s['terminal_actions'][1]=(s['terminal_actions'][1][0],'dispatched',None)
                        s['global_pending_stops']=[copy.deepcopy(f.stop)]
                    if fault in ('stop-attempt','stop-logical-action'):
                        action,_,receipt=s['terminal_actions'][1]
                        if fault=='stop-attempt':
                            action['identity']['attempt']=action['command']['attempt']=1
                        else:
                            action['identity']['logical_action']='effect-13'
                        receipt['identity']=copy.deepcopy(action['identity'])
                    if fault=='cleanup-saved':s['terminal_actions'][2]=(s['terminal_actions'][2][0],'receipted',{})
                    if fault=='cleanup-epoch':s['terminal_actions'][2][0]['identity']['epoch']+=1
                    if fault=='freeze-missing':s['original_actions']=[]
                    if fault=='extra-full':s['sources'][1]['capture']={'action':f.freeze,'worker':None,'completed':None,'end':None}
                    stack.enter_context(patch.object(frozen,'pidfd_exited',return_value=fault!='no-exit'))
                    stack.enter_context(patch.object(frozen,'original_process',return_value=fault=='still-live'))
                    sent=stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal',create=True))
                    queried=stack.enter_context(patch.object(frozen,'paused_source'))
                    if fault=='none':
                        value=p.assert_terminal_before_gc(p.deadline);self.assertTrue(value['original_source_exited'])
                        self.assertFalse(value['public_completion']);self.assertFalse(value['capacity_released'])
                        p.runtime.services.processes.start.assert_called_once_with('host-0')
                    elif fault in ('stop-attempt','stop-logical-action'):
                        with self.assertRaisesRegex(ValueError,'Cleanup does not retain the complete Stop'):
                            p.assert_terminal_before_gc(p.deadline)
                        p.runtime.services.processes.start.assert_not_called();self.assertIsNone(p.terminal_cut)
                    else:
                        with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                        p.runtime.services.processes.start.assert_not_called();self.assertIsNone(p.terminal_cut)
                    sent.assert_not_called();queried.assert_not_called()
                    self.assertEqual(p.continue_sent,p.continue_source_before_stop)

    def test_real_sql_snapshot_keeps_exact_history_global_stop_and_lifetime(self):
        with tempfile.TemporaryDirectory() as directory:
            f=fixture(Path(directory));p=f.probe;p._bind_destroy(f.accepted(f.destroy));raw=json.dumps(f.inbox).encode()
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER); CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB,owner_id TEXT); CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB); CREATE TABLE sandbox_lifetimes(sandbox_id TEXT,owner_id TEXT,started_at_ms INTEGER,deadline_ms INTEGER,max_deadline_ms INTEGER,startup_deadline_ms INTEGER);')
                conn.execute('CREATE TABLE controller_effect_keys(id INTEGER PRIMARY KEY,logical_json BLOB NOT NULL UNIQUE)')
                conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)',(raw,hashlib.sha256(raw).digest(),65536))
                name=p.config['sandbox'];conn.execute('INSERT INTO sandboxes VALUES(?,?,?)',(name,json.dumps(dict(f.snapshot['native'],owner=1)).encode(),'tenant-1'))
                conn.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?)',(name,'tenant-1',900,1000,1100,1000))
                logical=f.stop['command']['logical']
                conn.execute('INSERT INTO controller_effect_keys VALUES(3,?)',(json.dumps(logical).encode(),))
                for i,(a,state,receipt) in enumerate(f.snapshot['original_actions']+f.snapshot['terminal_actions']):
                    conn.execute('INSERT INTO outbox VALUES(?,?,?,?,?,?)',(i,name,a['identity']['operation'],json.dumps(a).encode(),state,None if receipt is None else json.dumps(receipt).encode()))
            Path(p.runtime.services.native_pools[0]['database']).chmod(0o600)
            with patch.object(capture,'checked_source_records',return_value=f.rows):
                got=p._read_terminal(p.deadline)
            self.assertEqual(got['original'],p.unload_entry);self.assertEqual(got['global_pending_stops'],[f.stop])
            self.assertEqual(got['lifetime'],p.prior['lifetime']);self.assertNotIn('resume_actions',got)
            self.assertIs(got['logical_action_registered'],True)
            for field,replacement in (('vm',10),('vm',9.0),('generation',2.0)):
                with self.subTest(field=field,replacement=replacement):
                    changed=copy.deepcopy(logical)
                    changed['command']['StopVm'][field]=replacement
                    with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                        conn.execute('UPDATE controller_effect_keys SET logical_json=?',(json.dumps(changed).encode(),))
                    with patch.object(capture,'checked_source_records',return_value=f.rows):
                        got=p._read_terminal(p.deadline)
                    self.assertIs(got['logical_action_registered'],False)
                    with self.assertRaisesRegex(ValueError,'complete logical effect registration'):
                        p._stop_barrier(got)
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.execute('UPDATE controller_effect_keys SET logical_json=?',(b' '*65537,))
            with patch.object(capture,'checked_source_records',return_value=f.rows), \
                    self.assertRaisesRegex(ValueError,'bounded original logical effect'):
                p._read_terminal(p.deadline)
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.execute('UPDATE controller_effect_keys SET id=4, logical_json=?',(json.dumps(logical).encode(),))
            with patch.object(capture,'checked_source_records',return_value=f.rows), \
                    self.assertRaisesRegex(ValueError,'bounded original logical effect'):
                p._read_terminal(p.deadline)
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.execute('UPDATE controller_effect_keys SET id=3')
                changed=copy.deepcopy(f.stop)
                changed['identity']['logical_action']='effect-03'
                conn.execute('UPDATE outbox SET action_json=? WHERE id=2',(json.dumps(changed).encode(),))
            with patch.object(capture,'checked_source_records',return_value=f.rows), \
                    self.assertRaisesRegex(ValueError,'canonical SQL logical action'):
                p._read_terminal(p.deadline)
            with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
                conn.execute('UPDATE outbox SET action_json=? WHERE id=2',(json.dumps(f.stop).encode(),))
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
    def endpoint(self,fault='none',*,state='Paused'):
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
            value=dict(app_name='Firecracker',id=resource,state=state,vmm_version='1.16.1')
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
            # The protected root-owned claim is an explicit controlled
            # provider here; Unix framing and procfs are real. Ordinary users
            # may have different UID/GID numbers. Check their real kernel
            # credentials before supplying the controlled VM role (UID=GID).
            # Ordinary tool tests must not require or simulate root ownership.
            def claim_provider(path,limit):
                self.assertEqual(path,claim/'record.json');self.assertEqual(limit,16384)
                return json.loads(path.read_bytes())
            testcase=self
            class RoleSocket(socket.socket):
                def getsockopt(self,level,option,*args):
                    actual=super().getsockopt(level,option,*args)
                    if (level,option)==(socket.SOL_SOCKET,socket.SO_PEERCRED):
                        testcase.assertEqual(struct.unpack('3i',actual),(os.getpid(),os.getuid(),os.getgid()))
                        if fault!='native-credentials':
                            return struct.pack('3i',os.getpid(),os.getuid(),os.getuid()+(fault=='gid'))
                    return actual
            try:
                with patch.object(frozen,'original_json',side_effect=claim_provider),patch.object(frozen.socket,'socket',RoleSocket):yield runner,prior,seen
            finally:thread.join(3);server.close();self.assertFalse(thread.is_alive());self.assertEqual(errors,[])

    def test_actual_unix_transport_and_controlled_vm_role_bind_paused_source(self):
        with self.endpoint() as (runner,prior,seen):
            result=frozen.paused_source(runner,prior,time.monotonic()+2)
            self.assertEqual(result['peer_pid'],os.getpid());self.assertEqual(result['state'],'Paused')
            self.assertFalse(result['execution_authority']);self.assertEqual(len(seen),1);self.assertTrue(seen[0].startswith(b'GET / HTTP/1.1'))

    def test_destroy_window_rejects_an_endpoint_replaced_after_the_paused_reply(self):
        for change in ('socket', 'jail'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory, ExitStack() as resources:
                # Commit the fixture before the mock server starts its bounded accept.
                f = fixture(Path(directory))
                freeze_window_journal(f)
                runner, prior, seen = resources.enter_context(self.endpoint())
                native = runner.services.native_pools[0]
                base = Path(native['chroot_base'])
                jail = base / 'firecracker' / prior['source']['resource_id'] / 'root'
                p = f.probe
                p.runtime.services.native_pools[0].update(native)
                p.prior['source'] = prior['source']
                p.prior['context']['position'] = prior['context']['position']
                p.frozen = frozen.paused_source(p.runtime, p.prior, p.deadline)

                with patch.object(capture, 'checked_source_records', return_value=f.rows), \
                        patch.object(p, '_check_source_hold'), \
                        patch.object(frozen.socket, 'socket', side_effect=AssertionError('unexpected VMM connection')):
                    before = p.observe_destroy_window(p.deadline)
                    self.assertEqual(p.observe_destroy_window(p.deadline), before)

                old_jail = jail.with_name('original-root')
                socket_path = jail / 'api.socket'
                if change == 'jail':
                    jail.rename(old_jail)
                    jail.mkdir()
                    (old_jail / socket_path.name).rename(socket_path)
                else:
                    socket_path.rename(jail / 'original.socket')
                with ExitStack() as stack:
                    if change == 'socket':
                        replacement = stack.enter_context(socket.socket(socket.AF_UNIX, socket.SOCK_STREAM))
                        descriptor = os.open(jail, os.O_RDONLY | os.O_DIRECTORY)
                        try:
                            replacement.bind(f'/proc/self/fd/{descriptor}/api.socket')
                        finally:
                            os.close(descriptor)
                    stack.enter_context(patch.object(capture, 'checked_source_records', return_value=f.rows))
                    stack.enter_context(patch.object(p, '_check_source_hold'))
                    stack.enter_context(patch.object(frozen.socket, 'socket',
                        side_effect=AssertionError('unexpected VMM connection')))
                    with self.assertRaisesRegex(ValueError, 'endpoint differs from the original Paused reply'):
                        p.observe_destroy_window(p.deadline)
                self.assertEqual(before['original_vmm_endpoint'], {
                    key: p.frozen[key] for key in ('jail_identity', 'socket_identity', 'uid')
                })
                self.assertEqual(len(seen), 1)

    def test_unmodified_kernel_credentials_require_both_vm_uid_and_gid(self):
        with self.endpoint('native-credentials') as (runner,prior,seen):
            if os.getuid()==os.getgid():
                self.assertEqual(frozen.paused_source(runner,prior,time.monotonic()+2)['peer_pid'],os.getpid())
                self.assertEqual(len(seen),1)
            else:
                with self.assertRaisesRegex(ValueError,'pause query peer differs'):
                    frozen.paused_source(runner,prior,time.monotonic()+2)
                self.assertEqual(seen,[])

    def test_running_wrong_peer_identity_version_and_framing_never_establish_frozen(self):
        for fault in ('running','peer','gid','version','resource','extra','length'):
            with self.subTest(fault=fault),self.endpoint(fault) as (runner,prior,seen),patch.object(frozen,'original_process',return_value=True):
                with self.assertRaises(ValueError):frozen.paused_source(runner,prior,time.monotonic()+2)


class FreezeStopWiringTests(unittest.TestCase):
    def test_actual_stop_socket_is_released_only_after_checks_and_peer_progress(self):
        for fault in ('none', 'peer-progress', 'global-stop', 'unregistered-stop',
                      'peer-stop-replacement', 'final-stop-replacement',
                      'ordered-none', 'ordered-peer-progress', 'ordered-global-stop',
                      'ordered-automatic-none', 'ordered-automatic-peer-progress',
                      'ordered-automatic-global-stop'):
            with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                ordered = fault.startswith('ordered-')
                fault = fault.removeprefix('ordered-')
                automatic=fault.startswith('automatic-')
                if automatic:
                    import lifecycle_freeze_expiry as automatic_expiry
                    from tools.tests.test_lifecycle_freeze_expiry import fixture as automatic_fixture
                    f=automatic_fixture(Path(directory));p=f.probe
                    if ordered:p.__class__=automatic_expiry.OrderedFreezeExpiryProbe
                    with patch.object(automatic_expiry,'now_ms',side_effect=[999,1000,1000,1000]):
                        accepted=p.observe_expiry(p.deadline)['acceptance']
                    recovery=p.recover_after_expiry
                    fault=fault.removeprefix('automatic-')
                else:
                    f=fixture(Path(directory));p=f.probe;accepted=f.accepted(f.destroy)
                    if ordered:p.__class__=frozen.OrderedFreezeDestroyProbe
                    recovery=p.recover_after_destroy
                events=[];received=[]
                server=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);server.bind(str(p.stop_path));server.listen(1);server.settimeout(2)
                def serve():
                    connection,_=server.accept()
                    with connection:received.append(connection.recv(1))
                thread=threading.Thread(target=serve);thread.start()
                host=Mock(pid=os.getpid());host.poll.return_value=None
                p.runtime.services.processes.start=Mock(side_effect=lambda *_,**__:events.append('start') or host)
                p.runtime.services.wait_ready=Mock();p._read_terminal=Mock(return_value=f.snapshot)
                if ordered:
                    accepted_snapshot=copy.deepcopy(f.snapshot)
                    fence=accepted_snapshot['terminal_actions'][0][0]
                    accepted_snapshot['terminal_actions']=[(fence,'ready',None)]
                    accepted_snapshot['global_pending_stops']=[]
                    accepted_snapshot['logical_action_registered']=False
                    p._read_terminal.side_effect=[accepted_snapshot,f.snapshot,f.snapshot,f.snapshot]
                    p.runtime.services.wait_ready.side_effect=lambda **_:events.append('wait-ready')
                if fault=='global-stop':f.snapshot['global_pending_stops']=[]
                if fault=='unregistered-stop':f.snapshot['logical_action_registered']=False
                if fault in ('peer-stop-replacement','final-stop-replacement'):
                    replaced=copy.deepcopy(f.snapshot)
                    stop=replaced['terminal_actions'][1][0]
                    if fault=='peer-stop-replacement':
                        stop['identity']['attempt']=stop['command']['attempt']=1
                        p._read_terminal.side_effect=[f.snapshot,replaced]
                    else:
                        stop['identity']['logical_action']='effect-13'
                        p._read_terminal.side_effect=[f.snapshot,f.snapshot,replaced]
                    replaced['global_pending_stops']=[copy.deepcopy(stop)]
                queried=stack.enter_context(patch.object(frozen,'paused_source',side_effect=lambda *_:events.append('paused') or {'original':'paused'}))
                p._source_state=Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                sent=stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal',side_effect=lambda *_:events.append('continue')))
                stack.enter_context(patch.object(frozen,'pidfd_exited',return_value=False))
                stack.enter_context(patch.object(frozen,'original_json',return_value=dict(epoch=4,terminal=True)))
                def progress():
                    self.assertFalse(p.stop_released);self.assertEqual(received,[]);events.append('peer')
                    if fault=='peer-progress':raise ValueError('unknown peer')
                    return {'original':'progress'}
                try:
                    if fault=='none':
                        result=recovery(accepted,p.deadline,observe_peer=progress)
                        self.assertTrue(result['original_pin_and_positive_charge_retained']);self.assertTrue(p.stop_released)
                        if automatic:
                            self.assertTrue(result['automatic_terminal_reached_host']);self.assertNotIn('public_destroy_reached_host',result)
                            p.client.destroy.assert_not_called()
                        if ordered:
                            self.assertLess(events.index('continue'),events.index('wait-ready'))
                            self.assertLess(events.index('continue'),events.index('peer'))
                            self.assertLess(events.index('peer'),events.index('paused'))
                            self.assertFalse(result['original_pidfd_continued_after_fence'])
                            self.assertTrue(result['source_continued_after_terminal_acceptance'])
                            self.assertEqual(result['terminal_acceptance_before_response_restored'],accepted_snapshot)
                            self.assertFalse(result['source_process_hold'])
                        else:
                            self.assertLess(events.index('peer'),events.index('continue'));self.assertLess(events.index('continue'),events.index('paused'))
                    elif fault in ('peer-stop-replacement','final-stop-replacement'):
                        with self.assertRaisesRegex(ValueError,'complete terminal Stop changed'):
                            recovery(accepted,p.deadline,observe_peer=progress)
                        self.assertFalse(p.stop_released);self.assertFalse(p.recovered)
                        self.assertEqual(events.count('peer'),1)
                        self.assertEqual(p.continue_sent,fault=='final-stop-replacement')
                        self.assertFalse(p.terminal_failure_state()['capacity_released'])
                    elif fault=='unregistered-stop':
                        with self.assertRaisesRegex(ValueError,'complete logical effect registration'):
                            recovery(accepted,p.deadline,observe_peer=progress)
                        self.assertFalse(p.stop_released);self.assertFalse(p.recovered)
                        self.assertFalse(p.continue_sent);self.assertNotIn('peer',events)
                    else:
                        with self.assertRaises(ValueError):recovery(accepted,p.deadline,observe_peer=progress)
                        self.assertFalse(p.stop_released);self.assertFalse(p.recovered)
                        if ordered:
                            self.assertTrue(p.continue_sent)
                            self.assertLess(events.index('continue'),events.index('wait-ready'))
                            failed = p.terminal_failure_state()
                            self.assertEqual(failed['terminal_acceptance_before_response_restored'],
                                             accepted_snapshot)
                            self.assertEqual(failed['automatic_terminal' if automatic else 'destroy'], accepted)
                            self.assertIs(failed['source_os_continue_sent'], True)
                            self.assertIs(failed['stop_gate_release_sent'], False)
                            self.assertIs(failed['capacity_released'], False)
                        else:
                            self.assertFalse(p.continue_sent);self.assertNotIn('continue',events)
                        self.assertFalse(p.terminal_failure_state()['capacity_released'])
                finally:
                    p.pidfd=None;p.close();thread.join(3);server.close();self.assertFalse(thread.is_alive())
                self.assertEqual(received,[b'g' if fault=='none' else b''])


class SceneAndReplayTests(unittest.TestCase):
    def test_new_resume_uses_current_destroy_version_and_requires_conflict(self):
        for fault in (None, 'not-terminating', 'accepted', 'unavailable', 'replaced'):
            with self.subTest(fault=fault):
                c = scene.PublicUnloadScenario('http://127.0.0.1:1', 'original', 'peer',
                                               'deployment', time.monotonic() + 30)
                c.session, c.window, c.sequence = 'session-1', 3, 8
                c.current_acceptance = dict(session=c.session, key=c.key(8), accepted_version=5)
                c.destroy_request = dict(window=3, sequence=8, expected_version=4)
                current = dict(session=c.session, operation=c.key(8), current_version=5,
                               phase='terminating', unload_outcome=None, create_refusal=None)
                first = dict(current, phase='cleaned') if fault == 'not-terminating' else current
                after = dict(current, current_version=6) if fault == 'replaced' else current
                status = {'accepted': 202, 'unavailable': 503}.get(fault, 409)
                replies = [(200, json.dumps(first)), (status, b''), (200, json.dumps(after))]
                with patch.object(scene, 'fetch', side_effect=replies) as fetch:
                    if fault in ('accepted', 'unavailable'):
                        with self.assertRaises(scene.PublicStatusMismatch) as error:
                            c.assert_terminal_rejects_new_resume()
                        self.assertEqual((error.exception.actual, error.exception.expected), (status, 409))
                    elif fault:
                        message = ('current Destroy must be terminating' if fault == 'not-terminating'
                                   else 'rejected new Resume changed the current Destroy')
                        with self.assertRaisesRegex(AssertionError, message):
                            c.assert_terminal_rejects_new_resume()
                    else:
                        self.assertEqual(c.assert_terminal_rejects_new_resume(), dict(
                            http_status=409, sequence=9, expected_version=5, terminal_view_unchanged=True))
                calls = [call.args[2:] for call in fetch.call_args_list]
                get = ('GET', c.path(), c.owner_key, None)
                post = ('POST', c.path() + '/resume', c.owner_key,
                        dict(window=3, sequence=9, expected_version=5))
                expected = [get] if fault == 'not-terminating' else [get, post]
                if fault in (None, 'replaced'):
                    expected.append(get)
                self.assertEqual(calls, expected)
                self.assertEqual(c.current_acceptance, dict(session=c.session, key=c.key(8), accepted_version=5))
                self.assertEqual(c.sequence, 8)

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

    def test_frozen_destroy_replay_matches_full_acceptance_and_terminal_view_at_both_edges(self):
        cases = (
            (None, None, 5),
            ('wrong-acceptance', 'Destroy replay changed the original acceptance', 2),
            ('changed-after-exact', 'exact Destroy replay changed the current terminal state', 3),
            ('unexpected-acceptance', 'public response status differs', 4),
            ('changed-after-altered', 'altered Destroy replay changed the current terminal state', 5),
        )
        for fault, message, count in cases:
            with self.subTest(fault=fault):
                client = scene.PublicUnloadScenario('http://127.0.0.1:1', 'original', 'peer',
                                                   'deployment', time.monotonic() + 30)
                client.session, client.window, client.sequence = 'session-1', 3, 8
                request = dict(window=3, sequence=8, expected_version=4)
                accepted = dict(session=client.session, key=client.key(8), accepted_version=5)
                current = dict(session=client.session, operation=accepted['key'], current_version=5,
                               phase='terminating', unload_outcome=None, create_refusal=None)
                client.destroy_request = copy.deepcopy(request)
                client.current_acceptance = copy.deepcopy(accepted)
                replay = dict(accepted, key=client.key(9)) if fault == 'wrong-acceptance' else accepted
                replaced = dict(current, operation=client.key(9))
                after_exact = replaced if fault == 'changed-after-exact' else current
                after_altered = replaced if fault == 'changed-after-altered' else current
                altered_status = 202 if fault == 'unexpected-acceptance' else 409
                replies = [(200, json.dumps(current)), (202, json.dumps(replay)),
                           (200, json.dumps(after_exact)), (altered_status, b''),
                           (200, json.dumps(after_altered))]
                with patch.object(scene, 'fetch', side_effect=replies) as fetch:
                    if message is None:
                        self.assertEqual(client.assert_frozen_destroy_replay(), dict(
                            original_acceptance_returned=True, altered_http_status=409,
                            altered_expected_version=5, terminal_view_unchanged=True))
                    else:
                        with self.assertRaisesRegex(AssertionError, message):
                            client.assert_frozen_destroy_replay()
                get = ('GET', client.path(), client.owner_key, None)
                post = ('POST', client.path() + '/destroy', client.owner_key)
                expected = [get, (*post, request), get,
                            (*post, dict(request, expected_version=5)), get]
                self.assertEqual([call.args[2:] for call in fetch.call_args_list], expected[:count])
                self.assertEqual(client.destroy_request, request)
                self.assertEqual(client.current_acceptance, accepted)
                self.assertEqual(client.sequence, 8)
                self.assertEqual(client.timings, [])

    def exercise(self, fault=False, peer_fault='none', expiry=False, window_fault=None, public_conflict=None):
        if expiry:
            import lifecycle_freeze_expiry as automatic
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
        def accept_destroy(_):
            events.append('destroy-1')
            clients['1'].current_acceptance = dict(session='session-1', accepted_version=5, key={'sequence':5})
            return copy.deepcopy(clients['1'].current_acceptance)
        original = clients['1']
        original.destroy.side_effect = accept_destroy
        veto_evidence = dict(http_status=409, sequence=6, expected_version=5, terminal_view_unchanged=True)
        replay_evidence = dict(original_acceptance_returned=True, altered_http_status=409,
                               altered_expected_version=5, terminal_view_unchanged=True)

        def check_public_conflict(step, evidence):
            events.append(step)
            if public_conflict == step:
                raise scene.PublicStatusMismatch(202, 409)
            return evidence

        original.assert_terminal_rejects_new_resume = Mock(
            side_effect=lambda: check_public_conflict('new-resume', veto_evidence))
        original.assert_frozen_destroy_replay = Mock(
            side_effect=lambda: check_public_conflict('destroy-replay', replay_evidence))
        from tools.tests.test_lifecycle_running_witness import RunningWitnessTests
        peer,peer_running=RunningWitnessTests().fixture()
        previous=copy.deepcopy(peer.witness);peer.witness=None;peer.data_token=None
        other=clients['2'];other.session=peer.session
        other.create_acceptance=copy.deepcopy(peer.create_acceptance)
        other.current_acceptance=copy.deepcopy(peer.current_acceptance);other.unload_acceptance=None
        other.wait_running.return_value=peer_running;other.query.return_value=peer_running
        def initialize(view):
            self.assertEqual(view,peer_running);events.append('run_public_workload-2')
            peer.witness=copy.deepcopy(previous);peer.data_token='same-execution'
            return dict(counter=previous['counter'])
        other.run_public_workload.side_effect=initialize
        real_running=peer.assert_running_workload;real_continued=peer.assert_continued_workload
        def progress(view):
            events.append('assert_running_workload-2');return real_running(view)
        other.assert_running_workload.side_effect=progress
        # Old wrong calls must reach the real failed-Unload preconditions.
        other.assert_continued_workload.side_effect=real_continued
        calls=0
        def running(session,*_,**__):
            nonlocal calls
            if session=='session-1':calls+=1
            return dict(session=session,lifetime=lifetime,context={'position':0 if session=='session-2' or calls==1 else 1})
        probe_type = automatic.OrderedFreezeExpiryProbe if expiry else frozen.OrderedFreezeDestroyProbe
        p = Mock(spec=probe_type)
        for name in ('arm','after_acceptance','assert_terminal_before_gc','cleanup','close'):
            getattr(p,name).side_effect=lambda *_,n=name,**__:events.append(n) or {}
        window_reads = iter(('before', 'after', 'after_rejected_resume', 'after_destroy_replay'))
        window_evidence = dict(freeze_sql_receipt_pending=True, original_pin_and_positive_charge_retained=True,
            original_vmm_endpoint=dict(jail_identity=[1, 2], socket_identity=[1, 3], uid=4))
        def observe_destroy_window(_):
            edge = next(window_reads)
            events.append('freeze-window-' + edge)
            if window_fault == edge:
                raise ValueError('original Freeze receipt no longer pending')
            return dict(window_evidence)
        p.observe_destroy_window.side_effect = observe_destroy_window
        def recovery(*_,observe_peer,**__):
            events.append('recover')
            if fault:raise ValueError('unknown original Stop')
            observe_peer();events.append('release-stop');return {}
        p.recover_after_destroy.side_effect=recovery
        if expiry:
            p.recover_after_expiry.side_effect=recovery
            p.observe_expiry.side_effect=lambda *_:events.append('automatic-expiry') or dict(
                acceptance={'observed':'internal'},observed_monotonic=time.monotonic())
        p.terminal_failure_state.return_value={'original':'preserved'}
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            sandbox=Mock();sandbox.files.read.side_effect=['disk-counter:3','disk-counter:4','disk-counter:4','disk-counter:5']
            sdk_calls=0
            def sdk():
                nonlocal sdk_calls
                sdk_calls+=1
                if peer_fault=='after-cleaned-token' and sdk_calls==2:peer.data_token='other-execution'
                return sandbox
            first=dict(previous,counter=3 if peer_fault=='held-counter' else 4)
            second=dict(previous,counter=5)
            stack.enter_context(patch.object(peer,'sdk',side_effect=sdk))
            witness=stack.enter_context(patch.object(peer,'witness_command',side_effect=[copy.deepcopy(previous),first,dict(previous,counter=4),second]))
            stack.enter_context(patch.object(peer,'query',return_value=peer_running))
            stack.enter_context(patch.object(peer,'assert_previous_tokens_closed'))
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
            module = automatic if expiry else frozen
            selected_probe = stack.enter_context(patch.object(module, probe_type.__name__, return_value=p))
            if expiry:
                stack.enter_context(patch.object(automatic, 'wait_expired_cleaned',
                                                 side_effect=lambda *_: events.append('wait_cleaned-1')))
            scenario = 'capture-freeze-receipt-expiry' if expiry else 'capture-freeze-receipt-destroy'
            args = NS(scenario=scenario, scenario_timeout=60, activation_gate_timeout_ms=60000)
            if public_conflict is not None or window_fault is not None:
                if public_conflict is not None:
                    with self.assertRaises(scene.PublicStatusMismatch):
                        scene.exercise(args)
                else:
                    with self.assertRaisesRegex(ValueError, 'original Freeze receipt no longer pending'):
                        scene.exercise(args)
                report = json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete'])
                phases = {
                    'new-resume': 'new-resume-after-public-destroy-before-host-reopen',
                    'destroy-replay': 'destroy-replay-before-host-reopen',
                    'before': 'recheck-original-freeze-before-public-destroy',
                    'after': 'recheck-original-freeze-after-public-destroy',
                    'after_rejected_resume': 'recheck-original-freeze-after-resume-veto',
                    'after_destroy_replay': 'recheck-original-freeze-after-destroy-replay',
                }
                self.assertEqual(report['phase'], phases[public_conflict or window_fault])
                if public_conflict is not None:
                    self.assertEqual(report['last_http_status'], dict(actual=202, expected=409))
                expected_windows = {
                    edge: window_evidence for edge in
                    ('before', 'after', 'after_rejected_resume', 'after_destroy_replay')
                    if 'freeze-window-' + edge in events and edge != window_fault
                }
                if window_fault == 'before':
                    original.destroy.assert_not_called()
                    self.assertNotIn('destroy', report)
                else:
                    original.destroy.assert_called_once()
                    self.assertEqual(report['destroy'], original.current_acceptance)
                    self.assertEqual(report['freeze_destroy_acceptance_window'], expected_windows)
                if public_conflict == 'destroy-replay' or window_fault in (
                        'after_rejected_resume', 'after_destroy_replay'):
                    self.assertEqual(report['frozen_destroy_resume_veto'], veto_evidence)
                else:
                    self.assertNotIn('frozen_destroy_resume_veto', report)
                if window_fault == 'after_destroy_replay':
                    self.assertEqual(report['frozen_destroy_replay'], replay_evidence)
                else:
                    self.assertNotIn('frozen_destroy_replay', report)
                other.destroy.assert_not_called()
                original.assert_terminal_unload_replay.assert_not_called()
                p.recover_after_destroy.assert_not_called()
                p.assert_terminal_before_gc.assert_not_called()
                runner.settlement_evidence.assert_not_called()
            elif peer_fault!='none':
                with self.assertRaises(AssertionError):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete']);self.assertEqual(report['failure_type'],'AssertionError')
                self.assertEqual(len(report['known_sessions']),2);other.destroy.assert_not_called()
                self.assertEqual(clients['1'].destroy.call_count,0 if expiry else 1)
                runner.settlement_evidence.assert_not_called()
                if peer_fault=='held-counter':
                    self.assertNotIn('release-stop',events);p.assert_terminal_before_gc.assert_not_called()
                    clients['1'].wait_cleaned.assert_not_called()
                    self.assertEqual(other.assert_running_workload.call_count,1)
                    self.assertEqual(report['phase'],'freeze-expiry-peer-progress-before-stop' if expiry else 'freeze-destroy-peer-progress-before-stop')
                else:
                    self.assertIn('release-stop',events)
                    if not expiry:clients['1'].wait_cleaned.assert_called_once()
                    self.assertEqual(other.assert_running_workload.call_count,2)
                    self.assertEqual(report['phase'],'freeze-expiry-peer-progress-after-cleaned' if expiry else 'freeze-destroy-peer-progress-after-cleaned')
                other.assert_continued_workload.assert_not_called()
            elif fault:
                with self.assertRaisesRegex(ValueError,'unknown original Stop'):scene.exercise(args)
                report=json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete']);self.assertEqual(len(report['known_sessions']),2)
                self.assertEqual(clients['1'].destroy.call_count,0 if expiry else 1);clients['2'].destroy.assert_not_called()
                p.assert_terminal_before_gc.assert_not_called();runner.settlement_evidence.assert_not_called()
                self.assertEqual(clients['1'].unload.call_count,2);self.assertEqual(clients['1'].resume.call_count,1)
                self.assertIn('freeze_expiry_failure_state' if expiry else 'freeze_destroy_failure_state',report)
            else:
                report=scene.exercise(args);self.assertTrue(report['complete']);self.assertEqual(len(report['cycles']),1)
                self.assertTrue(report['cycles'][0]['frozen_unload_expired' if expiry else 'frozen_unload_destroyed'])
                terminal_event='automatic-expiry' if expiry else 'destroy-1'
                self.assertLess(events.index('after_acceptance'),events.index(terminal_event))
                self.assertLess(events.index(terminal_event),events.index('recover'))
                if not expiry:
                    self.assertLess(events.index('freeze-window-before'),events.index(terminal_event))
                    self.assertLess(events.index(terminal_event),events.index('freeze-window-after'))
                    expected_order = [
                        'freeze-window-after', 'new-resume', 'freeze-window-after_rejected_resume',
                        'destroy-replay', 'freeze-window-after_destroy_replay', 'recover',
                    ]
                    self.assertEqual([event for event in events if event in expected_order], expected_order)
                    self.assertEqual(report['freeze_destroy_acceptance_window'], dict(
                        before=window_evidence, after=window_evidence, after_rejected_resume=window_evidence,
                        after_destroy_replay=window_evidence))
                    self.assertEqual(report['frozen_destroy_resume_veto'], veto_evidence)
                    self.assertEqual(report['frozen_destroy_replay'], replay_evidence)
                self.assertLess(events.index('assert_running_workload-2'),events.index('release-stop'))
                self.assertLess(events.index('release-stop'),events.index('assert_terminal_before_gc'))
                self.assertLess(events.index('assert_terminal_before_gc'),events.index('wait_cleaned-1'))
                self.assertLess(events.index('wait_cleaned-1'),events.index('destroy-2'))
                self.assertEqual(clients['1'].resume.call_count,1);self.assertIn('E8',report['acceptance_cases'])
            if expiry:
                p.observe_destroy_window.assert_not_called()
                original.assert_terminal_rejects_new_resume.assert_not_called()
                original.assert_frozen_destroy_replay.assert_not_called()
            if fault or window_fault is not None or public_conflict is not None:
                other.assert_running_workload.assert_not_called();witness.assert_not_called()
            elif peer_fault=='none':
                self.assertEqual(other.assert_running_workload.call_count,2)
                other.assert_continued_workload.assert_not_called()
                self.assertEqual([c.args[1] for c in witness.call_args_list],['read','advance','read','advance'])
                self.assertEqual(peer.witness,dict(previous,counter=5))
                self.assertEqual(report['slot_reuse']['peer_after_frozen_expiry' if expiry else 'peer_after_frozen_destroy']['counter'],5)
            if expiry:
                clients['1'].destroy.assert_not_called();clients['1'].wait_cleaned.assert_not_called()
                self.assertEqual(clients['1'].create.call_args.args[0],{'timeout_ms':75000})
                self.assertEqual(other.create.call_args.args[0],{})
                self.assertEqual(clients['1'].operation_evidence_completed.call_count,3)
                self.assertNotIn('destroy',report)
                if not fault and peer_fault=='none':
                    self.assertNotIn('final_destroy_send_to_settled_seconds',report['settlement_timing'])
                    self.assertGreaterEqual(report['settlement_timing']['expiry_observed_to_settled_seconds'],0)
                    self.assertLessEqual(report['automatic_expiry']['observed_monotonic'],time.monotonic())
                    self.assertEqual(report['freeze_response_mode'],
                                     'response-restored-after-terminal-acceptance')
            other.run_public_workload.assert_called_once_with(peer_running)
            self.assertEqual(clients['1'].unload.call_count,2);self.assertEqual(clients['1'].resume.call_count,1)
            selected_probe.assert_called_once()
            self.assertEqual(selected_probe.call_args.args[:2],(runner,clients['1']))
            runner.finish.assert_called_once();p.close.assert_called_once()
            self.assertLess(events.index('finish'),events.index('close'))

    def test_ordered_scene_keeps_original_terminal_peer_and_cleanup_checks(self):
        self.exercise()

    def test_ordered_scene_preserves_the_first_failure_at_either_destroy_window_edge(self):
        for edge in ('before', 'after'):
            with self.subTest(edge=edge):
                self.exercise(window_fault=edge)

    def test_ordered_scene_stops_if_fresh_resume_or_changed_destroy_is_accepted(self):
        for step in ('new-resume', 'destroy-replay'):
            with self.subTest(step=step):
                self.exercise(public_conflict=step)

    def test_ordered_scene_preserves_public_evidence_if_freeze_receipt_appears(self):
        for edge in ('after_rejected_resume', 'after_destroy_replay'):
            with self.subTest(edge=edge):
                self.exercise(window_fault=edge)

    def test_unknown_stop_keeps_original_failure_without_reissuing_or_destroying_peer(self):self.exercise(fault=True)

    def test_stalled_peer_before_stop_does_not_release_original_stop_or_destroy_peer(self):
        self.exercise(peer_fault='held-counter')

    def test_changed_peer_authorization_after_original_cleaned_does_not_destroy_peer(self):
        self.exercise(peer_fault='after-cleaned-token')


class OrderedFreezeTests(unittest.TestCase):
    def test_signal_failure_keeps_durable_kill_without_claiming_continuation(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            p = f.probe
            p.__class__ = frozen.OrderedFreezeDestroyProbe
            accepted = f.accepted(f.destroy)
            p._bind_destroy(accepted)
            self.assertNotIn('terminal_acceptance_before_response_restored', p.terminal_failure_state())
            expected = copy.deepcopy(f.snapshot)
            p._read_terminal = Mock(return_value=f.snapshot)
            p._source_state = Mock(return_value='T')
            first_error = OSError('could not continue the original VMM')
            with patch.object(frozen.signal, 'pidfd_send_signal', create=True,
                              side_effect=first_error) as sent:
                with self.assertRaises(OSError) as failure:
                    p._terminal_manager_started(p.deadline)
                self.assertIs(failure.exception, first_error)
                sent.assert_called_once_with(p.pidfd, signal.SIGCONT)

            # A later read or report consumer must not rewrite the saved cut.
            f.snapshot['native_kill_ledger']['result'] = 'Destroyed'
            failed = p.terminal_failure_state()
            self.assertIn('terminal_acceptance_before_response_restored', failed)
            self.assertEqual(failed['terminal_acceptance_before_response_restored'], expected)
            self.assertEqual(failed['destroy'], accepted)
            for field in ('source_os_continue_sent', 'stop_gate_release_sent',
                          'public_completion', 'capacity_released'):
                self.assertIs(failed[field], False)
            failed['terminal_acceptance_before_response_restored']['native_kill_ledger']['result'] = 'Destroyed'
            self.assertEqual(p.terminal_failure_state()['terminal_acceptance_before_response_restored'], expected)
            self.assertTrue(p.hold_source_process)

    def test_ordered_recovery_rejects_missing_or_different_kill_ledger(self):
        for fault in ('missing', 'foreign-owner', 'different-operation', 'different-generation',
                      'numeric-alias', 'result', 'completed-operation'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                f = fixture(Path(directory))
                p = f.probe
                p.__class__ = frozen.OrderedFreezeDestroyProbe
                p._bind_destroy(f.accepted(f.destroy))
                if fault == 'completed-operation':
                    f.snapshot['native']['current_operation']['status'] = 'Succeeded'
                    f.snapshot['native_kill_ledger']['operation']['status'] = 'Succeeded'
                path = ordered_kill_journal(f)
                ledger = copy.deepcopy(f.snapshot['native_kill_ledger'])
                with closing(sqlite3.connect(path)) as writer, writer:
                    if fault == 'missing':
                        writer.execute('DELETE FROM operations')
                    elif fault == 'foreign-owner':
                        writer.execute('UPDATE operations SET owner_id=?', ('tenant-2',))
                    else:
                        if fault == 'different-operation':
                            ledger['operation']['id'] += 1
                        elif fault == 'different-generation':
                            ledger['input']['Kill']['generation'] += 1
                        elif fault == 'numeric-alias':
                            ledger['input']['Kill']['generation'] = 2.0
                        elif fault == 'result':
                            ledger['result'] = 'Destroyed'
                        writer.execute('UPDATE operations SET state_blob=?', (json.dumps(ledger).encode(),))
                p._source_state = Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                with patch.object(capture, 'checked_source_records', return_value=f.rows), \
                        patch.object(frozen.signal, 'pidfd_send_signal', create=True) as sent:
                    with self.assertRaisesRegex(ValueError, 'original native Kill ledger'):
                        p._terminal_manager_started(p.deadline)
                sent.assert_not_called()
                p._source_state.assert_not_called()
                self.assertFalse(p.continue_sent)
                self.assertFalse(p.stop_released)

    def test_ordered_recovery_reads_kill_ledger_in_original_sql_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            f = fixture(root)
            p = f.probe
            p.__class__ = frozen.OrderedFreezeDestroyProbe
            p._bind_destroy(f.accepted(f.destroy))
            f.snapshot['native']['current_operation']['status'] = 'OutcomeUnknown'
            original = f.snapshot['native_kill_ledger']
            original['operation']['status'] = 'OutcomeUnknown'
            path = ordered_kill_journal(f)
            p._source_state = Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
            with closing(sqlite3.connect(path)) as writer:
                self.assertEqual(writer.execute('PRAGMA journal_mode=WAL').fetchone(), ('wal',))

                def finish_after_native_read(reader, _config, _deadline):
                    self.assertTrue(reader.in_transaction)
                    with writer:
                        writer.execute('UPDATE operations SET state_blob=?',
                                       (json.dumps(dict(original, result='Destroyed')).encode(),))
                    return f.rows

                with patch.object(capture, 'checked_source_records', side_effect=finish_after_native_read), \
                        patch.object(frozen.signal, 'pidfd_send_signal', create=True) as sent:
                    p._terminal_manager_started(p.deadline)
                    sent.assert_called_once_with(p.pidfd, signal.SIGCONT)
                self.assertEqual(p.response_restore_snapshot['native_kill_ledger'], original)
                self.assertTrue(p.continue_sent)
                self.assertFalse(p.stop_released)

            later = fixture(root).probe
            later.__class__ = frozen.OrderedFreezeDestroyProbe
            later._bind_destroy(f.accepted(f.destroy))
            with patch.object(capture, 'checked_source_records', return_value=f.rows), \
                    patch.object(frozen.signal, 'pidfd_send_signal', create=True) as sent:
                with self.assertRaisesRegex(ValueError, 'original native Kill ledger'):
                    later._terminal_manager_started(later.deadline)
                sent.assert_not_called()

    def test_response_stays_held_until_kill_and_original_freeze_outbox_agree(self):
        errors = {
            'stale-kill-epoch': 'durable exact original Kill',
            'replaced-freeze': 'original Freeze missing or replaced',
            'foreign-freeze-receipt': 'late original Freeze observation names another process',
            'original-seal': 'scheduled new capture or execution',
            'terminal-activate': 'scheduled new capture or execution',
        }
        for fault in (None, 'matching-freeze-receipt', *errors):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory, \
                    patch.object(frozen.signal, 'pidfd_send_signal', create=True) as sent:
                f = fixture(Path(directory))
                p = f.probe
                p.__class__ = frozen.OrderedFreezeDestroyProbe
                snapshot = copy.deepcopy(f.snapshot)
                fence = snapshot['terminal_actions'][0][0]
                snapshot['terminal_actions'] = [(fence, 'ready', None)]
                snapshot['global_pending_stops'] = []
                native = snapshot['native']
                if fault == 'stale-kill-epoch':
                    native['current_operation']['epoch'] = native['epoch'] = f.freeze['identity']['epoch']
                if fault == 'replaced-freeze':
                    snapshot['original_actions'][0][0]['identity']['logical_action'] = 'effect-99'
                if fault in ('matching-freeze-receipt', 'foreign-freeze-receipt'):
                    source = p.prior['source']
                    receipt = f.receipt(f.freeze, 'Frozen', dict(
                        pid=source['pid'] + (fault == 'foreign-freeze-receipt'),
                        start_ticks=source['start_time_ticks'], boot_id=source['boot_id']))
                    snapshot['original_actions'][0] = (f.freeze, 'receipted', receipt)
                if fault == 'original-seal':
                    seal = copy.deepcopy(f.freeze)
                    seal['identity']['logical_action'] = 'effect-99'
                    seal['command']['logical']['command'] = {'SealSnapshot': {}}
                    snapshot['original_actions'].append((seal, 'ready', None))
                if fault == 'terminal-activate':
                    activate = copy.deepcopy(fence)
                    activate['identity']['logical_action'] = 'effect-99'
                    activate['command']['logical']['command'] = {'ActivateVm': {}}
                    snapshot['terminal_actions'].append((activate, 'ready', None))
                p._read_terminal = Mock(return_value=snapshot)
                p._source_state = Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                if fault not in errors:
                    p._terminal_manager_started(p.deadline)
                    sent.assert_called_once_with(p.pidfd, signal.SIGCONT)
                    self.assertTrue(p.continue_sent)
                    self.assertFalse(p.hold_source_process)
                    self.assertEqual(p.response_restore_snapshot, snapshot)
                else:
                    with self.assertRaisesRegex(ValueError, errors[fault]):
                        p._terminal_manager_started(p.deadline)
                    sent.assert_not_called()
                    p._source_state.assert_not_called()
                    self.assertFalse(p.continue_sent)
                    self.assertTrue(p.hold_source_process)
                    self.assertNotIn('response_restore_snapshot', p.__dict__)
                self.assertFalse(p.stop_released)
                self.assertFalse(p.recovered)

    def test_numeric_aliases_cannot_restore_vmm_response_before_exact_kill(self):
        paths=(None,('generation',),('current_vm',),('net_slot',),('epoch',),
               ('current_operation','id'),('current_operation','epoch'),('both-epochs',))
        for path in paths:
            with self.subTest(path=path),tempfile.TemporaryDirectory() as directory, \
                    patch.object(frozen.signal,'pidfd_send_signal',create=True) as sent:
                f=fixture(Path(directory));p=f.probe;p.__class__=frozen.OrderedFreezeDestroyProbe
                snapshot=copy.deepcopy(f.snapshot);native=snapshot['native']
                if path==('both-epochs',):
                    native['epoch']=4.0;native['current_operation']['epoch']=4.0
                elif path is not None:
                    value=native
                    for key in path[:-1]:value=value[key]
                    value[path[-1]]=float(value[path[-1]])
                p._read_terminal=Mock(return_value=snapshot)
                p._source_state=Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                if path is None:
                    p._terminal_manager_started(p.deadline);sent.assert_called_once_with(p.pidfd,signal.SIGCONT)
                    self.assertTrue(p.continue_sent);self.assertFalse(p.hold_source_process)
                else:
                    with self.assertRaises(ValueError):p._terminal_manager_started(p.deadline)
                    sent.assert_not_called();self.assertFalse(p.continue_sent)
                    self.assertNotIn('response_restore_snapshot',p.__dict__)
                self.assertFalse(p.stop_released);self.assertFalse(p.recovered)

    # Controlled signal provider; real pidfd/Unix cases keep their Linux group.
    def test_response_restored_only_after_exact_durable_kill_and_retained_backing(self):
        for fault in ('none','wrong-vm','not-terminal','pin'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,patch.object(frozen.signal,'pidfd_send_signal',create=True) as sent:
                f=fixture(Path(directory));p=f.probe
                p.__class__=frozen.OrderedFreezeDestroyProbe
                p._read_terminal=Mock(return_value=f.snapshot)
                p._source_state=Mock(side_effect=lambda _: 'S' if p.continue_sent else 'T')
                if fault=='wrong-vm':f.snapshot['native']['current_vm']+=1
                if fault=='not-terminal':f.snapshot['native']['terminal']=False
                if fault=='pin':
                    for row in f.snapshot['sources']:
                        if row['restore'] is not None:row['restore']['material_use']=False
                if fault=='none':
                    p._terminal_manager_started(p.deadline)
                    sent.assert_called_once_with(p.pidfd,signal.SIGCONT)
                    self.assertTrue(p.continue_sent);self.assertFalse(p.hold_source_process)
                    self.assertFalse(p.stop_released)
                else:
                    with self.assertRaises(ValueError):p._terminal_manager_started(p.deadline)
                    sent.assert_not_called();self.assertFalse(p.continue_sent);self.assertFalse(p.stop_released)
