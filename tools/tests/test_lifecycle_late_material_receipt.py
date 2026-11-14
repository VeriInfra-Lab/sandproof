"""Real read-only SQLite plus controlled source/physical providers; no VM effects."""
import copy
from contextlib import ExitStack
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_late_material_receipt as late
import lifecycle_unload_scenario as scene
from tools.tests.test_lifecycle_unload_retirement import fixture, charge


class LateMaterialFixture(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.database = root/'db'
        self.records, self.original, _, _ = fixture()
        self.records[0]['retirement']['state'] = 'retiring'
        old = dict(position=0, expected_previous=None,
            binding=dict(owner=1, sandbox=1, generation=1, vm=1))
        context = dict(position=0, expected_previous=1,
            binding=dict(owner=2, sandbox=2, generation=1, vm=2))
        self.original['original_position'] = old
        self.occupant = dict(session='other-owner-session', context=context,
            local=dict(owner=2, sandbox=2), source=dict(resource_id='vm-original-2',
                pid=42, start_time_ticks=17, boot_id='original-boot'))
        native = dict(database=str(self.database), namespace='original')
        self.runtime = SimpleNamespace(services=SimpleNamespace(native_pools=[native],
            host_configs=[dict(native=dict(maintenance=dict(
                capacity=dict(max_sql_value_bytes=16384))))], processes=Mock()))
        self.state = dict(owner=2, phase='Running', terminal=False, generation=1, current_vm=2)
        with sqlite3.connect(self.database) as connection:
            connection.executescript('CREATE TABLE controller_host_slots(position,context_json);'
                'CREATE TABLE sandboxes(sandbox_id,state_blob);'
                'CREATE TABLE controller_data_credentials(sandbox_id,generation,digest);'
                'CREATE TABLE controller_data_credential_state(id,poisoned);'
                'CREATE TABLE sandbox_lifetimes(sandbox_id,owner_id,disk_bytes,started_at_ms,'
                'deadline_ms,max_deadline_ms,startup_deadline_ms);')
            connection.execute('INSERT INTO controller_host_slots VALUES(0,?)', (json.dumps(context),))
            connection.execute('INSERT INTO sandboxes VALUES(?,?)', ('sb-original-2',json.dumps(self.state)))
            connection.execute('INSERT INTO controller_data_credentials VALUES(?,?,?)',
                ('sb-original-2','1',b'x'*32))
            connection.execute('INSERT INTO controller_data_credential_state VALUES(1,0)')
            connection.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?,?)',
                ('sb-original-2','tenant-2',4096,1,100,200,20))
        # Existing strict source accounting and root-owned physical readers have
        # their own tests. These fixtures isolate this observer's composition.
        self.start_patch(patch.object(late, 'private_path', side_effect=Path))
        self.start_patch(patch.object(late, 'checked_source_records',
            side_effect=lambda *_: copy.deepcopy(self.records)))
        self.start_patch(patch.object(late, 'material_source_charge', side_effect=charge))
        self.physical = dict(source=copy.deepcopy(self.occupant['source']),
            disk=dict(device=1,inode=10), network=dict(device=2,inode=20),
            cgroup=dict(device=3,inode=30),backing=None)
        self.probe = late.LateMaterialReceiptObserver(self.runtime, session='original',
            original=self.original, occupant=self.occupant, deadline=time.monotonic()+20)
        self.reader = self.start_patch(patch.object(self.probe, '_physical',
            side_effect=lambda _: copy.deepcopy(self.physical)))

    def start_patch(self, patcher):
        self.addCleanup(patcher.stop)
        return patcher.start()

    def update(self, query, values=()):
        with sqlite3.connect(self.database) as connection:
            connection.execute(query, values)


class LateMaterialReceiptTests(LateMaterialFixture):
    def test_pending_then_same_plan_settled_preserves_other_owner_without_manager_calls(self):
        before = self.database.read_bytes()
        self.probe.freeze()
        self.assertEqual(self.probe.verify()['held_material_bytes'], 100)
        self.assertEqual(self.database.read_bytes(), before)
        self.records[0]['retirement']['state'] = 'settled'
        result = self.probe.verify()
        self.assertEqual(result['state'], 'settled')
        self.assertEqual(result['held_material_bytes'], 0)
        self.assertEqual(result['occupant_context'], self.occupant['context'])
        self.runtime.services.processes.assert_not_called()
        self.assertEqual(self.runtime.services.processes.mock_calls, [])
        with self.assertRaises(ValueError):
            self.probe.freeze()

    def test_retired_missing_or_replanned_original_cannot_refresh_baseline(self):
        self.probe.freeze()
        original = copy.deepcopy(self.records)
        for fault in ('missing','settlement-plan','reopened-pin','other-charge','old-history'):
            self.records[:] = copy.deepcopy(original)
            if fault == 'missing': self.records.pop(0)
            elif fault == 'settlement-plan': self.records[0]['retirement']['plan']['bytes'] = [8]
            elif fault == 'reopened-pin': self.records[0]['restore']['material_use'] = True
            elif fault == 'other-charge': self.records[1]['capture']['reserved_material_bytes'] += 1
            elif fault == 'old-history': self.records[0]['capture']['completed'] = [8]*32
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                self.probe.verify()
        self.records[:] = original
        self.assertEqual(self.probe.verify()['state'], 'retiring')

    def test_current_sql_generation_credential_slot_and_reservation_changes_are_rejected(self):
        self.probe.freeze()
        for key,value in [('generation',2),('current_vm',3),('owner',1),('phase','Destroyed'),
                          ('terminal',True)]:
            self.update('UPDATE sandboxes SET state_blob=?', (json.dumps(dict(self.state,**{key:value})),))
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.probe.verify()
        self.update('UPDATE sandboxes SET state_blob=?', (json.dumps(self.state),))
        for query,reset in [
            ("UPDATE controller_data_credentials SET digest=zeroblob(32)",
             'UPDATE controller_data_credentials SET digest=?'),
            ('UPDATE sandbox_lifetimes SET disk_bytes=2048','UPDATE sandbox_lifetimes SET disk_bytes=4096'),
            ('UPDATE controller_data_credential_state SET poisoned=1',
             'UPDATE controller_data_credential_state SET poisoned=0'),
            ('DELETE FROM controller_host_slots','INSERT INTO controller_host_slots VALUES(0,?)'),
        ]:
            self.update(query)
            with self.subTest(query=query), self.assertRaises(ValueError):
                self.probe.verify()
            values = (b'x'*32,) if 'digest=?' in reset else (
                (json.dumps(self.occupant['context']),) if 'VALUES' in reset else ())
            self.update(reset,values)
        self.assertEqual(self.probe.verify()['state'], 'retiring')

    def test_changed_physical_process_disk_network_or_cgroup_is_never_success(self):
        self.probe.freeze()
        original = copy.deepcopy(self.physical)
        for key in ('source','disk','network','cgroup','backing'):
            self.physical = copy.deepcopy(original)
            self.physical[key] = dict(replacement=True)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.probe.verify()

    def test_requires_actual_reuse_by_other_owner_and_pre_receipt_retiring_state(self):
        for field,value in [('position',1),('expected_previous',9)]:
            occupant = copy.deepcopy(self.occupant)
            occupant['context'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                late.LateMaterialReceiptObserver(self.runtime, session='original',
                    original=self.original,occupant=occupant,deadline=time.monotonic()+10)
        for field in ('owner','sandbox','vm'):
            occupant = copy.deepcopy(self.occupant)
            occupant['context']['binding'][field] = self.original['original_position']['binding'][field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                late.LateMaterialReceiptObserver(self.runtime, session='original',
                    original=self.original,occupant=occupant,deadline=time.monotonic()+10)
        with self.assertRaises(ValueError):
            self.probe.verify()
        self.records[0]['retirement']['state'] = 'settled'
        with self.assertRaises(ValueError):
            self.probe.freeze()

    def test_database_replacement_or_configuration_switch_cannot_move_the_baseline(self):
        self.probe.freeze()
        replacement = self.database.with_name('replacement')
        replacement.write_bytes(self.database.read_bytes())
        os.replace(replacement,self.database)
        with self.assertRaises(ValueError):
            self.probe.verify()


class LateReceiptGateTests(LateMaterialFixture):
    """Real readonly SQL/publication with a controlled reached-cut provider."""
    def setUp(self):
        super().setUp()
        self.events = []
        self.host = Mock()
        self.host.poll.return_value = None
        self.runtime.services.processes = SimpleNamespace(children={'host-0':self.host},
            stop=Mock(),start=Mock())
        self.runtime.lease = SimpleNamespace(identity='b'*64)
        self.runtime.gc_complete = self.runtime.physical_complete = False
        self.gate = late.LateMaterialReceiptProbe(self.runtime,'original',self.original,
            self.occupant,time.monotonic()+20)
        self.addCleanup(self.gate.close)
        self.gate.host = self.host
        self.gate.fd = os.open(self.directory.name,os.O_RDONLY|os.O_DIRECTORY)
        self.gate.observer._physical = Mock(side_effect=lambda _:copy.deepcopy(self.physical))
        self.gate._configuration = Mock()
        self.gate._unchanged = Mock()
        self.cut = dict(original=copy.deepcopy(self.records[0]),digest='a'*64,
            material=dict(original_work='work-A',original_plan='fixed-A'),
            observation=dict(phase='material-deleted-before-settlement'))
        self.gate.observe_deletion = Mock(return_value=self.cut)
        self.gate._save = Mock()
        self.counter = 1
        self.observe = Mock(side_effect=self.workload)
        original_publish = late.gate_publish
        self.settle_on_publish = True
        self.after_publish = lambda:None
        def publish(fd,name,value):
            self.assertEqual(self.events,['guest-2'])
            self.assertEqual(self.records[0]['retirement']['state'],'retiring')
            self.events.append('release')
            original_publish(fd,name,value)
            if self.settle_on_publish:
                self.records[0]['retirement']['state'] = 'settled'
            self.after_publish()
        self.publish = self.start_patch(patch.object(late,'gate_publish',side_effect=publish))

    def workload(self):
        self.counter += 1
        self.events.append('guest-'+str(self.counter))
        return dict(session=self.occupant['session'],same_ram_and_activation=True,
            same_guest_process=True,same_authorization=True,counter=self.counter,
            guest_pid=71,guest_start_time_ticks=123,ram_nonce_sha256='1'*64,
            activation_nonce_sha256='2'*64,metadata_generation=0)

    def test_original_receipt_stays_held_through_guest_io_then_same_plan_settles(self):
        result = self.gate.release_after_delete({'accepted_version':4},self.observe)
        self.assertEqual(self.events,['guest-2','release','guest-3'])
        command = json.loads((Path(self.directory.name)/'command.json').read_bytes())
        self.assertEqual(command,dict(schema_version=1,run_identity='b'*64,
            observation_sha256='a'*64,material=self.cut['material'],command='release'))
        self.assertEqual(result['settled_material_bytes'],0)
        self.assertTrue(result['same_host_manager_during_receipt_window'])
        self.assertTrue(result['late_stop_or_delete_not_exercised'])
        self.runtime.services.processes.stop.assert_not_called()
        self.runtime.services.processes.start.assert_not_called()
        with self.assertRaisesRegex(ValueError,'already completed'):
            self.gate.release_after_delete({},self.observe)
        self.gate.observe_deletion.assert_called_once()
        with self.assertRaises(ValueError):
            self.gate.cleanup()
        self.assertTrue((Path(self.directory.name)/'command.json').exists())

    def test_already_settled_or_replaced_host_never_releases_the_gate(self):
        for fault in ('settled','replaced','exited','cut-mismatch'):
            self.records[0]['retirement']['state'] = 'retiring'
            self.runtime.services.processes.children['host-0'] = self.host
            self.host.poll.return_value = None
            if fault == 'settled': self.records[0]['retirement']['state'] = 'settled'
            elif fault == 'replaced': self.runtime.services.processes.children['host-0'] = Mock()
            elif fault == 'exited': self.host.poll.return_value = 137
            else: self.cut['original']['retirement']['plan']['bytes'] = [0]
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                self.gate.release_after_delete({},self.observe)
            self.publish.assert_not_called()
            self.observe.assert_not_called()

    def test_failed_workload_or_early_delivery_cannot_be_a_held_window(self):
        for fault in ('guest-failure','early-settlement','wrong-owner'):
            self.records[0]['retirement']['state'] = 'retiring'
            self.counter = 1
            self.events.clear()
            # Independent fault histories need fresh observers; reusing the
            # first failed baseline would reject before reaching later faults.
            self.gate.observer = late.LateMaterialReceiptObserver(self.runtime,
                session='original',original=self.original,occupant=self.occupant,
                deadline=time.monotonic()+20)
            self.gate.observer._physical = Mock(side_effect=lambda _:copy.deepcopy(self.physical))
            def observe():
                value = self.workload()
                if fault == 'guest-failure': raise AssertionError('controlled guest failure')
                if fault == 'early-settlement': self.records[0]['retirement']['state'] = 'settled'
                if fault == 'wrong-owner': value['session'] = 'another-session'
                return value
            with self.subTest(fault=fault),self.assertRaises((ValueError,AssertionError)):
                self.gate.release_after_delete({},observe)
            self.assertEqual(self.events,['guest-2'])
            self.publish.assert_not_called()
            self.assertFalse(self.gate.recovered)

    def test_guest_identity_change_after_receipt_does_not_claim_acceptance(self):
        def observe():
            value = self.workload()
            if self.counter == 3: value['guest_pid'] += 1
            return value
        with self.assertRaisesRegex(ValueError,'guest identity changed'):
            self.gate.release_after_delete({},observe)
        self.assertFalse(self.gate.recovered)
        self.assertEqual(self.records[0]['retirement']['state'],'settled')

    def test_release_is_not_settlement_and_original_deadline_bounds_observation(self):
        self.settle_on_publish = False
        original_remaining = late.remaining
        released_reads = 0
        def remaining(deadline):
            nonlocal released_reads
            if 'release' in self.events:
                released_reads += 1
                if released_reads == 4: raise TimeoutError('original deadline')
            return original_remaining(deadline)
        with patch.object(late,'remaining',side_effect=remaining), \
                patch.object(late.time,'sleep'),self.assertRaises(TimeoutError):
            self.gate.release_after_delete({},self.observe)
        self.publish.assert_called_once()
        self.assertEqual(self.events,['guest-2','release'])
        self.assertEqual(charge(self.records[0]),100)
        self.assertFalse(self.gate.recovered)

    def test_shared_delete_cut_requires_original_plan_successor_consumption_and_absence(self):
        import lifecycle_unload_retirement as retirement
        records = copy.deepcopy(self.records)
        records[0]['capture']['full']['pause']['source_resource'] = {'original':'source-A'}
        records[1]['work']['requirement'] = dict(work_binding=[4]*32)
        records[0]['retirement']['plan']['closure'] = dict(reason=dict(kind='superseded',
            successor_work_binding=[4]*32,manifest_digest=records[1]['capture']['completed']))
        original = copy.deepcopy(self.original)
        missing = Path(self.directory.name)/'original-removed-material'
        original['material_paths'] = [str(missing)]
        key, digest = 'capture-'+'1'*64, 'a'*64
        with patch.object(retirement,'material_source_charge',side_effect=charge):
            expected = retirement.gate_material(records[0],key)
            for fault in ('none','phase','consumed','plan','successor','dangling-path'):
                rows = copy.deepcopy(records)
                observation = dict(schema_version=1,run_identity='b'*64,
                    phase='material-deleted-before-settlement',material=copy.deepcopy(expected),
                    source_resource=rows[0]['capture']['full']['pause']['source_resource'])
                consumed = dict(schema_version=1,run_identity='b'*64,
                    observation_sha256=digest,material=copy.deepcopy(expected))
                if fault == 'phase': observation['phase'] = 'before-delete'
                elif fault == 'consumed': consumed['material']['work_binding'] = 'c'*64
                elif fault == 'plan': rows[0]['retirement']['plan']['plan_binding'] = [9]*32
                elif fault == 'successor': rows[0]['retirement']['plan']['closure']['reason']['manifest_digest'] = [9]*32
                elif fault == 'dangling-path': missing.symlink_to('missing-target')
                probe = retirement.RetirementRestartProbe(self.runtime,'original',original,time.monotonic()+10)
                probe.config = dict(capture_key=key)
                probe._wait_ready = Mock(return_value=dict(schema_version=1,observation_sha256=digest))
                probe._save = Mock(side_effect=[(observation,digest),(consumed,'c'*64)])
                probe._unchanged, probe._configuration = Mock(),Mock()
                self.runtime._materials = Mock(return_value=dict(manifest={'original':True}))
                with self.subTest(fault=fault),patch.object(retirement,'_read_snapshot',return_value=(rows,rows[1])):
                    if fault == 'none':
                        result = probe.observe_deletion({'accepted_version':4})
                        self.assertEqual(result['material'],expected)
                    else:
                        with self.assertRaises(ValueError): probe.observe_deletion({})
        self.runtime.services.processes.stop.assert_not_called()
        self.runtime.services.native_pools[0]['namespace'] = 'another'
        with self.assertRaises(ValueError):
            self.probe.verify()


class OtherOwnerWorkloadTests(unittest.TestCase):
    def client(self, fault=None):
        client = scene.PublicUnloadScenario('http://127.0.0.1:1','owner','foreign',
            'deployment',time.monotonic()+20,owner='2')
        client.session, client.data_token = 'occupant','original-credential'
        client.witness = dict(nonce='1'*64,activation_nonce='2'*64,
            metadata_generation=0,counter=1,pid=71,start_time_ticks=123)
        current = copy.deepcopy(client.witness)
        def command(text,**_):
            if text.endswith(' advance'):
                current['counter'] += 1
                if fault == 'guest-pid': current['pid'] += 1
            elif not text.endswith(' read'):
                self.fail('only original witness read/advance expected')
            if fault == 'ram': current['nonce'] = '3'*64
            return SimpleNamespace(exit_code=0,stdout=json.dumps(current))
        sdk = SimpleNamespace(commands=SimpleNamespace(run=Mock(side_effect=command)),
            files=SimpleNamespace(read=Mock(side_effect=lambda _: 'wrong' if fault=='disk'
                else 'disk-counter:'+str(current['counter']))))
        def connect():
            if fault == 'credential': client.data_token = 'replacement-credential'
            return sdk
        client.sdk = Mock(side_effect=connect)
        running = dict(phase='running',session=client.session)
        client.query = Mock(return_value=running)
        client.assert_previous_tokens_closed = Mock()
        return client, sdk, running

    def test_original_guest_and_credential_continue_with_two_observed_advances(self):
        client, sdk, running = self.client()
        before = client.assert_running_workload(running)
        after = client.assert_running_workload(running)
        self.assertEqual((before['counter'],after['counter']),(2,3))
        self.assertEqual(before['guest_pid'],after['guest_pid'])
        self.assertEqual(before['ram_nonce_sha256'],after['ram_nonce_sha256'])
        self.assertEqual(sdk.commands.run.call_count,4)
        self.assertEqual(client.data_token,'original-credential')

    def test_changed_guest_ram_disk_or_credential_cannot_claim_continuity(self):
        for fault in ('guest-pid','ram','disk','credential'):
            client,_,running = self.client(fault)
            with self.subTest(fault=fault),self.assertRaises(AssertionError):
                client.assert_running_workload(running)


class LateReceiptScenarioWiringTests(unittest.TestCase):
    """Drive the actual selector with controlled public/physical providers."""
    def run_scene(self, scenario, *, fail_gate=False):
        import lifecycle_unload_cow as cow
        import lifecycle_unload_retirement as retirement
        import lifecycle_unload_runtime as runtime
        events, destroyed = [], set()
        running = dict(phase='running')
        lifetime = dict(started_at_ms=1,end_at_ms=100,max_end_at_ms=200,startup_deadline_ms=20)
        clients = {}
        for owner in ('1','2'):
            client = Mock(spec=scene.PublicUnloadScenario)
            client.owner, client.session = owner,'session-'+owner
            client.current_acceptance = dict(key={'sequence':1},accepted_version=1)
            client.unload_acceptance = dict(key={'sequence':2},accepted_version=2)
            client.concurrent_unload_replies = [client.unload_acceptance]*2
            client.resume_request, client.witness = {},{}
            client.last_http_status, client.failure_step = None,'controlled-provider'
            client.timings = [dict(sent_monotonic=time.monotonic())]
            client.timing_report.return_value = []
            client.wait_running.return_value = running
            client.wait_unloaded.return_value = dict(phase='unloaded')
            client.query.return_value = running
            client.create.side_effect = lambda _,o=owner:events.append('create-'+o) or {}
            client.resume.side_effect = lambda _,o=owner:events.append('resume-'+o) or {'accepted_version':3}
            client.unload.side_effect = lambda *_,o=owner,**__:events.append('unload-'+o) or {'accepted_version':4}
            client.destroy.side_effect = lambda _,o=owner:destroyed.add(o) or events.append('destroy-'+o) or {}
            client.run_public_workload.side_effect = lambda _,o=owner:events.append('workload-'+o) or {}
            client.assert_running_workload.side_effect = lambda _,o=owner:events.append('held-workload-'+o) or {}
            client.assert_resumed_workload.return_value = {}
            clients[owner] = client
        counts = dict(running=0,unloaded=0)
        def record(session,_,**__):
            if session == 'session-1': counts['running'] += 1
            position = 0 if session=='session-2' or counts['running']==1 else 1
            return dict(session=session,context=dict(position=position),lifetime=lifetime)
        def unloaded(*_):
            counts['unloaded'] += 1
            return dict(original_position=dict(position=0 if counts['unloaded']<=2 else 1),
                facts=dict(lifetime),original_capture_responsibilities=1)
        def settle(*_,**__):
            events.append('source-settled')
            return dict(receipt={},snapshot={})
        def quota(_):
            self.assertEqual(destroyed,{'1','2'})
            events.append('zero-quota')
            return dict(remaining=0)
        gate = Mock()
        gate.cleanup.return_value = 'exact-test-gate-cleanup'
        gate.arm.side_effect = lambda _:events.append('arm') or {}
        def release(_,observe):
            self.assertNotIn('2',destroyed)
            events.append('receipt-held')
            observe()
            if fail_gate: raise ValueError('controlled receipt failure')
            events.append('receipt-release')
            observe()
            return dict(same_plan_settled=True)
        gate.release_after_delete.side_effect = release
        gate.restart_after_delete.side_effect = lambda _:events.append('gate-restart') or {}
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            runner = SimpleNamespace(root=Path(directory),owner_key='owner',foreign_key='foreign',
                services=SimpleNamespace(sdk_url='http://127.0.0.1:1'),
                measurement_identity=lambda:{},start=Mock(),create_parameters=lambda **_:{},
                record_running=Mock(side_effect=record),unload_evidence=Mock(side_effect=unloaded),
                restart_managers=Mock(return_value=dict(original_unload_operation={})),
                destroy_evidence=Mock(side_effect=lambda session,_:events.append('evidence-'+session) or {}),
                settlement_evidence=Mock(side_effect=quota),finish=Mock(),
                fail_resume_gate=Mock(return_value='preserved'),
                close_restore_gate=Mock(),close_capture_cancel_gate=Mock())
            factory = stack.enter_context(patch.object(scene,'PublicUnloadScenario',
                side_effect=lambda *_,owner='1':clients[owner]))
            stack.enter_context(patch.object(scene,'check_center_configuration',return_value={'deployment':'original'}))
            stack.enter_context(patch.object(runtime,'prepare',return_value=runner))
            sealed = Mock()
            sealed.verify_after_guest_write.return_value = {}
            stack.enter_context(patch.object(cow,'SealedSourceProbe',return_value=sealed))
            stack.enter_context(patch.object(retirement,'wait_settled',side_effect=settle))
            stack.enter_context(patch.object(retirement,'assert_replay_unchanged'))
            stack.enter_context(patch.object(retirement,'RetirementRestartProbe',return_value=gate))
            stack.enter_context(patch.object(late,'LateMaterialReceiptProbe',return_value=gate))
            # These are provider-boundary fixtures, not simulated Rust policy or
            # physical certificates. The real observer/guest checks are above.
            runner.confirm_running = Mock(return_value={})
            args = SimpleNamespace(scenario=scenario,scenario_timeout=60,activation_gate_timeout_ms=60000)
            if fail_gate:
                with self.assertRaises(ValueError): scene.exercise(args)
                report = json.loads((Path(directory)/'lifecycle-unload-report.json').read_bytes())
                self.assertFalse(report['complete'])
                self.assertNotIn('2',destroyed)
                gate.cleanup.assert_not_called()
                self.assertEqual(len(report['known_sessions']),2)
            else:
                report = scene.exercise(args)
                self.assertTrue(report['complete'])
                self.assertEqual(factory.call_count,2)
                self.assertEqual(events.count('destroy-1'),1)
                self.assertEqual(events.count('destroy-2'),1)
                self.assertLess(events.index('destroy-2'),events.index('zero-quota'))
            return report,events,gate

    def test_new_selector_keeps_other_owner_until_receipt_but_old_selectors_keep_order(self):
        for scenario in ('normal','material-retirement-restart','material-retirement-late-receipt'):
            with self.subTest(scenario=scenario):
                report,events,gate = self.run_scene(scenario)
                if scenario == 'material-retirement-late-receipt':
                    self.assertLess(events.index('receipt-held'),events.index('destroy-2'))
                    self.assertLess(events.index('receipt-release'),events.index('source-settled'))
                    self.assertLess(events.index('source-settled'),events.index('destroy-2'))
                    self.assertEqual(events.count('held-workload-2'),2)
                    self.assertIn('E9',report['acceptance_cases'])
                    gate.restart_after_delete.assert_not_called()
                else:
                    self.assertLess(events.index('destroy-2'),events.index('source-settled'))
                    gate.release_after_delete.assert_not_called()

    def test_failed_new_window_preserves_report_and_never_claims_cleanup_or_acceptance(self):
        self.run_scene('material-retirement-late-receipt',fail_gate=True)

    def test_invalid_gate_budget_is_rejected_before_preparing_resources(self):
        import lifecycle_unload_runtime as runtime
        for budget in (True,999,120001,1.0):
            args = SimpleNamespace(scenario='material-retirement-late-receipt',
                scenario_timeout=60,activation_gate_timeout_ms=budget)
            with self.subTest(budget=budget),patch.object(runtime,'prepare') as prepare, \
                    self.assertRaises(ValueError):
                scene.exercise(args)
            prepare.assert_not_called()

    def test_real_cli_lists_the_explicit_new_selector(self):
        result = subprocess.run([sys.executable,scene.__file__,'--help'],
            capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('material-retirement-late-receipt',result.stdout)


class LinuxLateMaterialPhysicalReaderTests(LateMaterialFixture):
    def test_physical_reader_uses_real_process_and_files_with_controlled_claim_provider(self):
        root = Path(self.directory.name)
        source = dict(resource_id='vm-original-2',pid=os.getpid(),
            start_time_ticks=late.process_start_ticks(os.getpid()),
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
        native = self.probe.native
        native.update(chroot_base=str(root/'v'),firecracker='/fixed/firecracker',
            host_resources=dict(cgroup_parent=str(root/'groups')))
        self.runtime.services.native_pools[0] = copy.deepcopy(native)
        self.probe.occupant['source'] = source
        claims = root/'v/.claims'/source['resource_id']
        claims.mkdir(parents=True)
        (claims/'record.json').write_text(json.dumps(source))
        namespace = root/'namespace'
        namespace.touch()
        identity = late._identity
        stamp = identity(namespace)
        network = dict(resource_id=source['resource_id'],
            namespace_device=stamp['device'],namespace_inode=stamp['inode'])
        (claims/'network.json').write_text(json.dumps(network))
        group = root/'groups'/source['resource_id']
        group.mkdir(parents=True)
        (group/'cgroup.procs').write_text(str(source['pid']))
        disk = root/'v/firecracker'/source['resource_id']/'root/rootfs.ext4'
        disk.parent.mkdir(parents=True)
        disk.write_bytes(b'fixed test disk')
        # Controlled claim provider avoids claiming privileged root ownership.
        # Process checks and the disk/cgroup/namespace inode reads remain real.
        with patch.object(late,'original_json',side_effect=lambda path,_:json.loads(path.read_text())), \
                patch.object(late,'_identity',side_effect=lambda path,**kw:identity(
                    namespace if Path(path).parent == Path('/run/netns') else path,**kw)):
            read = lambda:late.LateMaterialReceiptObserver._physical(self.probe,self.records)
            observed = read()
            self.assertEqual(observed['source'],source)
            self.assertEqual(observed['disk']['inode'],disk.stat().st_ino)
            with patch.object(late,'process_start_ticks',return_value=None), self.assertRaises(ValueError):
                read()
            (group/'cgroup.procs').write_text('')
            with self.assertRaises(ValueError): read()
            (group/'cgroup.procs').write_text(str(source['pid']))
            (claims/'network.json').write_text(json.dumps(dict(network,namespace_inode=0)))
            with self.assertRaises(ValueError): read()
            (claims/'network.json').write_text(json.dumps(network))
            restored = copy.deepcopy(self.records[0])
            restored['work']['anchor']['local']['sandbox'] = 2
            restored['restore'].update(generation=1,material_use=True)
            self.records.append(restored)
            with self.assertRaises(ValueError): read()
            backing = dict(manifest=dict(sha256=restored['capture']['completed']))
            self.probe.occupant['running_backing'] = backing
            self.runtime._materials = Mock(return_value=copy.deepcopy(backing))
            self.assertEqual(read()['backing'],backing)
            restored['restore']['material_use'] = False
            with self.assertRaises(ValueError): read()
            restored['restore']['material_use'] = True
            self.runtime._materials.return_value = dict(replacement=True)
            with self.assertRaises(ValueError): read()


if __name__ == '__main__':
    unittest.main()
