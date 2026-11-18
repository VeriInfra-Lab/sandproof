"""Original audit DTO and inherited frozen consumers; no VM or clock changes."""
import copy
from contextlib import ExitStack, closing
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
import lifecycle_freeze_expiry as expiry
import lifecycle_freeze_destroy as frozen
import lifecycle_unload_capture_faults as capture
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
    p.observe_destroy_window = Mock(return_value={'freeze_sql_receipt_pending': True})
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


def host_history(f):
    _, create, _ = transport.InternalFaultRelayTests.pair(None)
    return dict(id=f.probe.client.session, current_version=4, terminal=False,
                entries=[dict(intent=copy.deepcopy(i)) for i in [create]+f.intents])


def write_history(f, row):
    """Seed the provider before observation; the consumer opens it read-only."""
    database=Path(f.probe.runtime.services.native_pools[0]['database'])
    raw=json.dumps(dict(sessions=[row]),separators=(',',':')).encode()
    with sqlite3.connect(database) as conn:
        conn.execute('CREATE TABLE IF NOT EXISTS controller_lifecycle_inbox(id INTEGER PRIMARY KEY,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER)')
        conn.execute('INSERT OR REPLACE INTO controller_lifecycle_inbox VALUES(1,?,?,?)',
                     (raw,hashlib.sha256(raw).digest(),65536))
    database.chmod(0o600)
    return database


class FrozenExpiryHistoryTests(unittest.TestCase):
    """Same old/new after_acceptance contract, real SQLite, controlled facts."""
    def fresh(self, root):
        f=fixture(root)
        del f.probe.expiry_baseline
        f.history=host_history(f)
        f.processes.audit.return_value=f.before
        return f

    def observe(self, f):
        with patch.object(frozen.FreezeDestroyProbe,'after_acceptance',return_value={'original':'Freeze'}), \
                patch.object(expiry,'now_ms',return_value=950):
            return f.probe.after_acceptance(f.probe.accepted,f.probe.deadline)

    def unchanged_responsibility(self, f, acceptance, sources, database, before):
        self.assertEqual(f.probe.client.current_acceptance,acceptance)
        self.assertEqual(f.probe.initial_sources,sources)
        self.assertEqual(database.read_bytes(),before)
        self.assertIsNone(f.probe.automatic_acceptance)
        self.assertFalse(f.probe.stop_released)
        self.assertFalse(f.probe.continue_sent)
        self.assertIsNone(f.probe.terminal_accepted)
        f.probe.client.query.assert_not_called()
        f.probe.client.destroy.assert_not_called()

    def test_original_four_operation_history_is_readonly_and_keeps_number_gaps(self):
        with tempfile.TemporaryDirectory() as directory:
            f=self.fresh(Path(directory))
            f.history['entries'].reverse()  # Ledger order is not version order.
            database=write_history(f,f.history);before=database.read_bytes()
            acceptance=copy.deepcopy(f.probe.client.current_acceptance)
            sources=copy.deepcopy(f.probe.initial_sources)
            result=self.observe(f)
            self.assertTrue(result['boundary_before_real_expiry'])
            self.assertEqual(f.probe.expiry_baseline,f.before['sandboxes'][0])
            self.assertEqual(f.intents[1]['request']['operation']['target_incarnation'],7)
            self.assertNotEqual(f.intents[1]['retention']['window'],f.intents[1]['request']['binding']['key']['window'])
            self.unchanged_responsibility(f,acceptance,sources,database,before)
            f.processes.audit.assert_called_once_with('center')

    def test_each_original_context_and_raw_options_are_preserved_without_a_serde_model(self):
        with tempfile.TemporaryDirectory() as directory:
            f=self.fresh(Path(directory));create=f.history['entries'][0]['intent']
            create['center']['center']='original-create-center'
            create['retention'].update(slot=31,window=91)
            payload=create['request']['operation']['create'];payload['options']=list(b'{ }')
            first=f.before['sandboxes'][0]['operations'][0]
            first['center']=copy.deepcopy(create['center']);first['retention']=copy.deepcopy(create['retention'])
            f.before['sandboxes'][0]['create']['options_sha256']=hashlib.sha256(bytes(payload['options'])).hexdigest()
            # A valid diagnostic metadata digest has no content-equality claim.
            f.before['sandboxes'][0]['create']['metadata_sha256']='e'*64
            database=write_history(f,f.history);before=database.read_bytes()
            acceptance=copy.deepcopy(f.probe.client.current_acceptance);sources=copy.deepcopy(f.probe.initial_sources)
            self.assertTrue(self.observe(f)['boundary_before_real_expiry'])
            self.unchanged_responsibility(f,acceptance,sources,database,before)

    def test_replaced_create_envelope_or_projection_cannot_become_the_expiry_baseline(self):
        faults=('center','deployment','slot','retention-window','extra-context','extra-retention',
                'slot-bool','slot-float','protocol','node-id','node-generation','node-deployment','expected',
                'extra-binding','target-incarnation','extra-operation','template-revision','template-digest',
                'instances','cpu','memory','disk','resource-bool','resource-float','options-digest',
                'startup-deadline','max-lifetime','extra-create','metadata-shape')
        for fault in faults:
            with tempfile.TemporaryDirectory() as directory,self.subTest(fault=fault):
                f=self.fresh(Path(directory));row=f.before['sandboxes'][0]
                op=row['operations'][0];binding=op['binding'];shape=row['create']
                if fault=='center':op['center']['center']='replaced'
                if fault=='deployment':op['center']['deployment']='other'
                if fault=='slot':op['retention']['slot']+=1
                if fault=='retention-window':op['retention']['window']+=1
                if fault=='extra-context':op['center']['extra']=1
                if fault=='extra-retention':op['retention']['extra']=1
                if fault=='slot-bool':op['retention']['slot']=True
                if fault=='slot-float':op['retention']['slot']=float(op['retention']['slot'])
                if fault=='protocol':binding['protocol_version']=3
                if fault=='node-id':binding['node']['node_id']='other'
                if fault=='node-generation':binding['node']['node_generation']+=1
                if fault=='node-deployment':binding['node']['deployment_id']='other'
                if fault=='expected':binding['expected']={'state':'version','version':0}
                if fault=='extra-binding':binding['extra']=1
                if fault=='target-incarnation':op['operation']['target_incarnation']=2
                if fault=='extra-operation':op['operation']['capture']=3
                if fault=='template-revision':shape['template_revision']='other'
                if fault=='template-digest':shape['template_digest'][0]^=1
                if fault in ('instances','cpu'):shape['resources'][fault]+=1
                if fault=='memory':shape['resources']['memory_bytes']+=1
                if fault=='disk':shape['resources']['disk_bytes']+=1
                if fault=='resource-bool':shape['resources']['instances']=True
                if fault=='resource-float':shape['resources']['cpu']=float(shape['resources']['cpu'])
                if fault=='options-digest':shape['options_sha256']='a'*64
                if fault=='startup-deadline':shape['startup_deadline_ms']+=1
                if fault=='max-lifetime':shape['max_lifetime_ms']+=1
                if fault=='extra-create':shape['extra']=1
                if fault=='metadata-shape':shape['metadata_sha256']='unbounded-diagnostic'
                shape['binding']=copy.deepcopy(binding)  # Joint replacement defeats self-correlation.
                database=write_history(f,f.history);before=database.read_bytes()
                acceptance=copy.deepcopy(f.probe.client.current_acceptance);sources=copy.deepcopy(f.probe.initial_sources)
                with self.assertRaises(ValueError):self.observe(f)
                self.assertFalse(hasattr(f.probe,'expiry_baseline'))
                self.unchanged_responsibility(f,acceptance,sources,database,before)

    def test_same_json_value_with_different_original_option_bytes_is_not_the_same_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            f=self.fresh(Path(directory))
            f.history['entries'][0]['intent']['request']['operation']['create']['options']=list(b'{ }')
            database=write_history(f,f.history);before=database.read_bytes()
            acceptance=copy.deepcopy(f.probe.client.current_acceptance);sources=copy.deepcopy(f.probe.initial_sources)
            with self.assertRaises(ValueError):self.observe(f)
            self.assertFalse(hasattr(f.probe,'expiry_baseline'))
            self.unchanged_responsibility(f,acceptance,sources,database,before)

    def test_wrong_host_prefix_is_rejected_before_the_center_manager_is_touched(self):
        faults=('missing','extra','duplicate','terminal','current-version','create-session','create-key',
                'create-version','create-node','create-protocol','create-expected','create-source',
                'old-unload-context','old-unload-capture','resume-context','resume-target','current-context','current-capture')
        for fault in faults:
            with tempfile.TemporaryDirectory() as directory,self.subTest(fault=fault):
                f=self.fresh(Path(directory));row=f.history;entries=row['entries']
                create,unload,resume,current=[e['intent'] for e in entries]
                if fault=='missing':entries.pop(0)
                if fault=='extra':entries.append(copy.deepcopy(entries[0]))
                if fault=='duplicate':entries[1]=copy.deepcopy(entries[0])
                if fault=='terminal':row['terminal']=True
                if fault=='current-version':row['current_version']=3
                if fault=='create-session':create['request']['binding']['session']='other'
                if fault=='create-key':create['request']['binding']['key']['sequence']+=10
                if fault=='create-version':create['accepted_version']=2
                if fault=='create-node':create['request']['binding']['node']['node_id']='other'
                if fault=='create-protocol':create['request']['binding']['protocol_version']=3
                if fault=='create-expected':create['request']['binding']['expected']={'state':'version','version':0}
                if fault=='create-source':create['request']['operation']['target_incarnation']=2
                if fault=='old-unload-context':unload['center']['center']='other'
                if fault=='old-unload-capture':unload['request']['operation']['capture']+=1
                if fault=='resume-context':resume['retention']['window']+=1
                if fault=='resume-target':resume['request']['operation']['target_incarnation']=8
                if fault=='current-context':current['center']['center']='other'
                if fault=='current-capture':current['request']['operation']['capture']+=1
                database=write_history(f,row);before=database.read_bytes()
                acceptance=copy.deepcopy(f.probe.client.current_acceptance);sources=copy.deepcopy(f.probe.initial_sources)
                with self.assertRaises(ValueError):self.observe(f)
                self.assertFalse(hasattr(f.probe,'expiry_baseline'))
                f.processes.stop.assert_not_called();f.processes.start.assert_not_called();f.processes.audit.assert_not_called()
                self.unchanged_responsibility(f,acceptance,sources,database,before)


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
                write_history(f,host_history(f))
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
                events.append('clock');return 999 if events.count('clock') <= 2 else 1000
            p.observe_destroy_window.side_effect = lambda _: events.append('freeze-window') or {'held': True}
            f.processes.stop.side_effect=lambda *_:events.append('stop-center')
            f.processes.audit.side_effect=lambda *_:events.append('audit') or f.after
            f.processes.start.side_effect=lambda *_:events.append('start-center')
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=clock))
            result=p.observe_expiry(p.deadline)
            self.assertEqual(events, ['clock', 'freeze-window', 'clock', 'clock', 'stop-center',
                                      'audit', 'start-center', 'clock', 'clock', 'freeze-window'])
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
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,999,1000,1000,1000,1000]))
            sleep=stack.enter_context(patch.object(expiry.time,'sleep'))
            result=p.observe_expiry(p.deadline)
            self.assertEqual(result['center_audits'],2);sleep.assert_called_once()
            self.assertEqual(f.processes.start.call_count,2)

    def test_expiry_audit_brackets_the_original_unacknowledged_freeze_in_sqlite(self):
        for fault in (None, 'receipt-before-expiry', 'receipt-after-audit'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                f = fixture(Path(directory))
                p = f.probe
                del p.observe_destroy_window
                database = original.freeze_window_journal(f)
                f.processes.children['host-0'] = p.host
                endpoint = original.controlled_frozen_endpoint(p)
                stack.enter_context(patch.object(capture, 'checked_source_records', return_value=f.rows))
                stack.enter_context(patch.object(expiry, 'now_ms', side_effect=[999, 999, 1000, 1000, 1000]))

                def save_receipt():
                    source = p.prior['source']
                    receipt = f.receipt(f.freeze, 'Frozen', dict(pid=source['pid'],
                        start_ticks=source['start_time_ticks'], boot_id=source['boot_id']))
                    with closing(sqlite3.connect(database)) as conn, conn:
                        conn.execute("UPDATE outbox SET dispatch_state='receipted',receipt_json=? WHERE id=1",
                                     (json.dumps(receipt).encode(),))

                def audit(_):
                    if fault == 'receipt-after-audit':
                        save_receipt()
                    return f.after

                f.processes.audit.side_effect = audit
                if fault == 'receipt-before-expiry':
                    save_receipt()
                if fault is None:
                    result = p.observe_expiry(p.deadline)
                    window = result['freeze_receipt_window']
                    self.assertEqual(window['before'], window['after'])
                    self.assertTrue(window['after']['freeze_sql_receipt_pending'])
                    self.assertEqual(window['after']['host_nonterminal_pause'], original.pending_pause(f)['current_operation'])
                    self.assertEqual(window['after']['original_vmm_endpoint'], endpoint.return_value)
                    self.assertEqual(endpoint.call_count, 2)
                    f.processes.audit.assert_called_once_with('center')
                else:
                    with self.assertRaisesRegex(ValueError, 'one exact dispatched action with no SQL receipt'):
                        p.observe_expiry(p.deadline)
                    failure = p.terminal_failure_state()
                    if fault == 'receipt-before-expiry':
                        f.processes.audit.assert_not_called()
                        self.assertIsNone(failure['observed_automatic_acceptance'])
                        self.assertEqual(p.client.current_acceptance, p.accepted)
                    else:
                        f.processes.audit.assert_called_once_with('center')
                        f.processes.start.assert_called_once_with('center')
                        self.assertEqual(failure['observed_automatic_acceptance'], f.accepted)
                        self.assertEqual(set(failure['freeze_receipt_window']), {'before'})
                        self.assertEqual(p.client.current_acceptance, f.accepted)
                    with self.assertRaisesRegex(ValueError, 'only the audited automatic terminal acceptance'):
                        p._bind_destroy(f.accepted)
                    self.assertIsNone(p.terminal_accepted)
                    self.assertFalse(failure['source_os_continue_sent'])
                    self.assertFalse(failure['stop_gate_release_sent'])
                p.client.query.assert_not_called()
                p.client.destroy.assert_not_called()

    def test_explicit_destroy_early_expiry_or_dead_center_cannot_certify_autonomous_expiry(self):
        for fault in ('destroy','early','window-crossed-expiry','center','host','source','changed-current'):
            with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe
                if fault=='destroy':p.client.destroy_request={}
                if fault=='center':f.center.poll.return_value=1
                if fault=='host':p.host.poll.return_value=0
                if fault=='source':p._source_state.return_value='R'
                if fault=='changed-current':p.client.current_acceptance={}
                times = {'early': [1000], 'window-crossed-expiry': [999, 1000]}
                stack.enter_context(patch.object(expiry, 'now_ms', side_effect=times.get(fault, [999, 999, 1000])))
                with self.subTest(fault=fault),self.assertRaises(ValueError):p.observe_expiry(p.deadline)
                f.processes.audit.assert_not_called();p.client.query.assert_not_called();p.client.destroy.assert_not_called()
                self.assertIsNone(p.automatic_acceptance);self.assertFalse(p.stop_released)
                if fault == 'window-crossed-expiry':
                    self.assertEqual(set(p.terminal_failure_state()['freeze_receipt_window']), {'before'})

    def test_audit_failure_restarts_center_and_keeps_original_unload_and_hold(self):
        with tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
            f=fixture(Path(directory));p=f.probe;prior=copy.deepcopy(p.client.current_acceptance)
            f.processes.audit.side_effect=ValueError('invalid original audit')
            stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,999,1000]))
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


class OrderedFrozenExpiryTests(unittest.TestCase):
    # Supply the controlled provider even where Python exposes no Linux pidfd.
    def test_only_audited_unchanged_automatic_acceptance_can_start_terminal_host(self):
        for probe_type in (expiry.OrderedFreezeExpiryProbe,expiry.UnresponsiveFreezeExpiryProbe):
            for fault in ('unobserved','destroy','key','current'):
                with self.subTest(probe=probe_type.__name__,fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                    f=fixture(Path(directory));p=f.probe;p.__class__=probe_type
                    if fault!='unobserved':
                        stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,999,1000,1000,1000]))
                        p.observe_expiry(p.deadline)
                    value=copy.deepcopy(f.accepted)
                    if fault=='destroy':p.client.destroy_request={}
                    if fault=='key':value['key']['sequence']+=1
                    if fault=='current':p.client.current_acceptance['accepted_version']+=1
                    sent=stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal',create=True))
                    starts=list(f.processes.start.call_args_list)
                    progress=Mock()
                    with self.assertRaisesRegex(ValueError,'only the audited automatic terminal acceptance'):
                        p.recover_after_expiry(value,p.deadline,observe_peer=progress)
                    self.assertEqual(f.processes.start.call_args_list,starts);p._configuration.assert_not_called()
                    progress.assert_not_called();sent.assert_not_called()
                    self.assertIsNone(p.terminal_accepted);self.assertFalse(p.stop_released)

    def test_exact_host_kill_and_backing_required_before_response_without_waiting_for_stop(self):
        for fault in ('none','vm','generation','network','phase','terminal','operation','epoch','ledger','pin','material','commit'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory,ExitStack() as stack:
                f=fixture(Path(directory));p=f.probe;p.__class__=expiry.OrderedFreezeExpiryProbe
                endpoint=original.controlled_frozen_endpoint(p)
                stack.enter_context(patch.object(expiry,'now_ms',side_effect=[999,999,1000,1000,1000]))
                accepted=p.observe_expiry(p.deadline)['acceptance'];p._bind_destroy(accepted)
                snapshot=copy.deepcopy(f.snapshot)
                fence=snapshot['terminal_actions'][0][0]
                snapshot['terminal_actions']=[(fence,'ready',None)]
                snapshot['global_pending_stops']=[]
                native=snapshot['native']
                if fault=='vm':native['current_vm']+=1
                if fault=='generation':native['generation']+=1
                if fault=='network':native['net_slot']+=1
                if fault=='phase':native['phase']='Pausing'
                if fault=='terminal':native['terminal']=False
                if fault=='operation':native['current_operation']['id']+=1
                if fault=='epoch':native['current_operation']['epoch']+=1
                if fault=='ledger':snapshot['native_kill_ledger']['result']='Destroyed'
                if fault=='pin':snapshot['sources'][0]['restore']['material_use']=False
                if fault=='material':p.runtime._materials.return_value={'changed':'backing'}
                if fault == 'commit':
                    commit = copy.deepcopy(snapshot['original_actions'][0][0])
                    commit['identity']['logical_action'] = 'effect-99'
                    commit['command']['logical']['command'] = {'CommitSnapshot': {}}
                    snapshot['original_actions'].append((commit, 'ready', None))
                sent=stack.enter_context(patch.object(frozen.signal,'pidfd_send_signal',create=True))
                sleep=stack.enter_context(patch.object(frozen.time,'sleep'))
                def not_yet_host_accepted():
                    sent.assert_not_called();self.assertFalse(p.continue_sent)
                    raise frozen._AwaitTerminal('original Host Kill not yet admitted')
                reads=iter((not_yet_host_accepted,lambda:snapshot))
                p._read_terminal=Mock(side_effect=lambda _, *, extra:next(reads)())
                p._source_state=Mock(side_effect=lambda _:'S' if p.continue_sent else 'T')
                if fault=='none':
                    p._terminal_manager_started(p.deadline)
                    sent.assert_called_once_with(p.pidfd,frozen.signal.SIGCONT)
                    self.assertTrue(p.continue_sent);self.assertFalse(p.hold_source_process)
                    self.assertEqual(p.response_restore_snapshot,snapshot)
                    self.assertEqual(p.response_restore_endpoint,endpoint.return_value)
                    endpoint.assert_called_once_with(p.deadline)
                    self.assertEqual(snapshot['original_actions'],f.snapshot['original_actions'])
                    self.assertEqual(snapshot['global_pending_stops'],[])
                else:
                    with self.assertRaises(ValueError):p._terminal_manager_started(p.deadline)
                    sent.assert_not_called();self.assertFalse(p.continue_sent)
                    endpoint.assert_not_called()
                    self.assertTrue(p.hold_source_process)
                sleep.assert_called_once();self.assertFalse(p.stop_released)
                self.assertEqual([call.kwargs['extra'] for call in p._read_terminal.call_args_list],
                                 [p._read_kill_ledger, p._read_kill_ledger])
                p.client.destroy.assert_not_called()

    def test_scene_keeps_audited_expiry_and_original_peer_cleanup_checks(self):
        original.SceneAndReplayTests().exercise(expiry=True,ordered_expiry=True)

    def test_unknown_stop_preserves_both_sessions_and_audited_acceptance(self):
        original.SceneAndReplayTests().exercise(expiry=True,ordered_expiry=True,fault=True)

    def test_stalled_peer_keeps_stop_held_after_response_restoration(self):
        original.SceneAndReplayTests().exercise(expiry=True,ordered_expiry=True,peer_fault='held-counter')

    def test_changed_peer_authorization_keeps_peer_after_automatic_cleanup(self):
        original.SceneAndReplayTests().exercise(expiry=True,ordered_expiry=True,peer_fault='after-cleaned-token')


class UnresponsiveFrozenExpiryTests(unittest.TestCase):
    def test_scene_keeps_automatic_expiry_and_original_peer_cleanup_checks(self):
        original.SceneAndReplayTests().exercise(expiry=True,unresponsive=True)

    def test_unknown_stop_preserves_both_sessions_and_automatic_acceptance(self):
        original.SceneAndReplayTests().exercise(expiry=True,unresponsive=True,fault=True)

    def test_stalled_peer_keeps_stop_held_without_restoring_response(self):
        original.SceneAndReplayTests().exercise(expiry=True,unresponsive=True,peer_fault='held-counter')

    def test_changed_peer_authorization_keeps_peer_after_automatic_cleanup(self):
        original.SceneAndReplayTests().exercise(expiry=True,unresponsive=True,peer_fault='after-cleaned-token')
