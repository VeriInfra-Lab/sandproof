"""E5 consumers with original SQL/Unix primitives and controlled facts; no KVM."""
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
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tests/system'))
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_scenario as scene
import lifecycle_unload_stop_pending as pending
import lifecycle_unload_runtime as runtime
from tools.tests import test_lifecycle_freeze_destroy as original


def fixture(root):
    f = original.fixture(root)
    p = pending.PendingStopProbe.__new__(pending.PendingStopProbe)
    p.__dict__.update(f.probe.__dict__)
    p.stage = 'stop-receipt'
    p.pending = None
    p.stop_released = False
    p.stop_peer = None
    p.client.current_acceptance = copy.deepcopy(p.running_acceptance)
    p.client.unload_request = dict(window=p.accepted['key']['window'], sequence=p.accepted['key']['sequence'], expected_version=3)
    p.host.pid = os.getpid()
    p.host.poll.return_value = None
    p.client.query.return_value = dict(session=p.client.session, operation=p.accepted['key'],
        current_version=4, phase='unloading', unload_outcome={'state': 'pending'}, create_refusal=None)
    p.client.check_view.side_effect = lambda view, accepted: scene.PublicUnloadScenario.check_view(p.client, view, accepted)
    stop = copy.deepcopy(f.stop)
    stop['identity']['operation'] = 'op-' + f.runner.services.native_pools[0]['namespace'] + '-4-4'
    stop['identity']['epoch'] = 3
    stop['identity']['logical_action'] = 'effect-4'
    stop['command']['logical']['command']['StopVm']['epoch'] = 3
    work = f.rows[1]
    requirement = work['work']['requirement']
    requirement['work_binding'] = [8] * 32
    full = dict(state_bytes=1, memory_bytes=2, disk_bytes=3, pause=dict(
        binding=dict(operation=copy.deepcopy(f.unload['request']['binding']), source_incarnation=7, capture=5),
        source=copy.deepcopy(requirement['source']), source_resource=copy.deepcopy(requirement['resource']),
        source_work_binding=requirement['work_binding'], node=f.node))
    work['capture'] = dict(operation=4, full=full, reserved_material_bytes=2*6+2*262144+65536,
        action={'original': 'seal'}, worker={'original': 'producer'}, completed=[9]*32, end=None)
    for key in ('action', 'worker'):
        f.rows[0]['capture'][key] = {'original': 'backing-' + key}
    f.runner._materials.return_value = {'original': 'material'}
    extra = dict(global_pending_stops=[stop], logical_action_registered=True,
        lifetime=copy.deepcopy(p.prior['lifetime']),
        native=dict(owner=1, generation=2, current_vm=9, epoch=3, terminal=False))
    snapshot = (copy.deepcopy(p.unload_entry), f.rows, [(stop, 'dispatched', None)], extra)
    return NS(probe=p, parent=f, snapshot=snapshot, stop=stop, rows=f.rows)


class PendingFactsTests(unittest.TestCase):
    def test_same_original_request_stop_material_and_backing_are_required(self):
        mutations = [
            lambda s: s[0]['intent']['center'].update(center='another-center'),
            lambda s: s[0]['intent']['request']['binding']['expected'].update(version=2),
            lambda s: s[0].update(unload_resolution={}),
            lambda s: s[2][0][0]['identity'].update(attempt=1),
            lambda s: s[3]['global_pending_stops'].append(copy.deepcopy(s[3]['global_pending_stops'][0])),
            lambda s: s[3].update(logical_action_registered=False),
            lambda s: s[3]['native'].update(terminal=True),
            lambda s: s[3]['native'].update(owner=True),
            lambda s: s[3]['native'].update(current_vm=10),
            lambda s: s[3]['lifetime'].update(end_at_ms=1001),
            lambda s: s[1][1]['work']['requirement']['source'].update(pid=124),
            lambda s: s[1][1]['work']['requirement']['resource'].update(kind='disk'),
            lambda s: s[1][1]['capture'].update(completed=None),
            lambda s: s[1][1]['capture'].update(reserved_material_bytes=0),
            lambda s: s[1][0]['restore'].update(material_use=False),
            lambda s: s[1][0]['restore']['intent']['request']['operation'].update(target_incarnation=8),
            lambda s: s[2].append((dict(command={'logical': {'command': {'CleanupVm': {'vm': 9}}}}), 'ready', None)),
        ]
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            good = f.probe._facts(f.snapshot)
            self.assertEqual(good['stop'], f.stop)
            for index, mutate in enumerate(mutations):
                changed = copy.deepcopy(f.snapshot)
                mutate(changed)
                with self.subTest(index=index), self.assertRaises((ValueError, KeyError)):
                    f.probe._facts(changed)

    def test_complete_nine_field_action_and_typed_command_are_not_partial_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory))
            for field in f.stop['identity']:
                changed = copy.deepcopy(f.stop)
                del changed['identity'][field]
                with self.subTest(missing=field), self.assertRaises(ValueError):
                    f.probe._stop_shape(changed)
            for field in ('epoch', 'attempt', 'protocol_version'):
                changed = copy.deepcopy(f.stop)
                changed['identity'][field] = True
                with self.subTest(boolean=field), self.assertRaises(ValueError):
                    f.probe._stop_shape(changed)

    def test_current_public_unload_never_accepts_completed_terminal_or_unknown(self):
        with tempfile.TemporaryDirectory() as directory:
            p = fixture(Path(directory)).probe
            p._public_pending()
            p.client.assert_data_closed.assert_called_once()
            p.client.assert_previous_tokens_closed.assert_called_once()
            for phase in ('unloaded', 'running', 'terminating', 'cleaned', 'state_lost'):
                view = copy.deepcopy(p.client.query.return_value)
                view['phase'] = phase
                with self.subTest(phase=phase):
                    p.client.query.return_value = view
                    with self.assertRaises((ValueError, AssertionError)):
                        p._public_pending()
            p.client.query.side_effect = TimeoutError('unknown original query')
            with self.assertRaises(TimeoutError):
                p._public_pending()

    def test_default_capture_hooks_keep_old_environment_and_three_value_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            p = fixture(Path(directory)).probe
            fault = {'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'stop_vm'}
            self.assertEqual(capture.CaptureHandoffProbe._fault_environment(p), fault)
            self.assertIsNone(capture.CaptureHandoffProbe._before_observation(p, p.deadline))
            self.assertEqual(p._fault_environment(), dict(fault, SANDPROOF_TEST_STOP_GATE=str(p.stop_path)))
            with patch.object(capture.CaptureHandoffProbe, '_fault_environment', return_value={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'freeze_vm'}), self.assertRaises(ValueError):
                p._fault_environment()

    def test_original_post_stop_assertion_is_still_mandatory_after_release(self):
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            f = fixture(Path(directory)); p = f.probe
            p.accepted = None
            p.host.wait.return_value = 86
            def held(_):
                p.pending = {'original_stop': copy.deepcopy(f.stop)}
                p.stop_released = True
            p._before_observation = Mock(side_effect=held)
            p._read = Mock(return_value=f.snapshot[:3])
            stack.enter_context(patch.object(capture, 'pidfd_exited', return_value=False))
            with self.assertRaisesRegex(ValueError, 'Stop did not close'):
                p.after_acceptance(f.parent.accepted(f.parent.unload), p.deadline)
            self.assertTrue(p.stop_released)

    def test_success_requires_same_original_post_stop_identity_and_consumed_socket(self):
        with tempfile.TemporaryDirectory() as directory:
            p = fixture(Path(directory)).probe
            p.runtime.physical_complete = True
            for recovered, released, gc in [(False, True, True), (True, False, True), (True, True, False)]:
                p.recovered, p.stop_released, p.runtime.gc_complete = recovered, released, gc
                with self.subTest(recovered=recovered, released=released, gc=gc), self.assertRaises(ValueError):
                    p.cleanup()
            p.recovered = p.stop_released = p.runtime.gc_complete = True
            self.assertTrue(p.cleanup()['original_consumed_stop_socket_absent'])
            p.stop_path.write_bytes(b'unknown')
            with self.assertRaises(ValueError):
                p.cleanup()
            self.assertEqual(p.stop_path.read_bytes(), b'unknown')

    def test_live_source_checks_precede_actual_pause_query_and_never_signal(self):
        for fault in ('none', 'host-exit', 'source-exit', 'source-identity'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                p = fixture(Path(directory)).probe
                if fault == 'host-exit': p.host.poll.return_value = 86
                stack.enter_context(patch.object(pending, 'pidfd_exited', return_value=fault == 'source-exit'))
                stack.enter_context(patch.object(pending, 'original_process', return_value=fault != 'source-identity'))
                paused = stack.enter_context(patch.object(pending, 'paused_source', return_value={'actual': 'paused'}))
                if fault == 'none':
                    self.assertEqual(p._live(p.deadline), {'actual': 'paused'})
                    paused.assert_called_once_with(p.runtime, p.prior, p.deadline)
                else:
                    with self.assertRaises(ValueError): p._live(p.deadline)
                    paused.assert_not_called()


class SameSqlSnapshotTests(unittest.TestCase):
    def test_original_sql_snapshot_binds_global_stop_native_lifetime_and_logical_effect(self):
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            f = fixture(Path(directory)); p = f.probe
            name = p.config['sandbox']; operation = f.stop['identity']['operation']
            inbox = dict(sessions=[dict(id=p.client.session, current_version=4, terminal=False,
                local=p.prior['local'], entries=[f.snapshot[0]])])
            raw = json.dumps(inbox).encode()
            database = p.runtime.services.native_pools[0]['database']
            with sqlite3.connect(database) as conn:
                conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);'
                    'CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB);'
                    'CREATE TABLE controller_effect_keys(id INTEGER,logical_json BLOB);'
                    'CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB,owner_id TEXT);'
                    'CREATE TABLE sandbox_lifetimes(sandbox_id TEXT,owner_id TEXT,started_at_ms INTEGER,deadline_ms INTEGER,max_deadline_ms INTEGER,startup_deadline_ms INTEGER);')
                conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,?)', (raw, hashlib.sha256(raw).digest(), 65536))
                conn.execute('INSERT INTO outbox VALUES(1,?,?,?,\'dispatched\',NULL)', (name, operation, json.dumps(f.stop).encode()))
                conn.execute('INSERT INTO controller_effect_keys VALUES(4,?)', (json.dumps(f.stop['command']['logical']).encode(),))
                conn.execute('INSERT INTO sandboxes VALUES(?,?,?)', (name, json.dumps(f.snapshot[3]['native']).encode(), 'tenant-1'))
                conn.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?)', (name, 'tenant-1', 900, 1000, 1100, 1000))
            stack.enter_context(patch.object(capture, 'private_path', side_effect=lambda value, **_: Path(value)))
            stack.enter_context(patch.object(capture, 'checked_source_records', return_value=f.rows))
            selected = p._snapshot(p.deadline)
            self.assertEqual(p._facts(selected)['stop'], f.stop)
            self.assertEqual(len(capture.CaptureHandoffProbe._read(p, p.deadline)), 3)
            with sqlite3.connect(database) as conn:
                changed = copy.deepcopy(f.stop['command']['logical'])
                changed['command']['StopVm']['vm'] = 10
                conn.execute('UPDATE controller_effect_keys SET logical_json=?', (json.dumps(changed).encode(),))
            with self.assertRaises(ValueError):
                p._facts(p._snapshot(p.deadline))
            with sqlite3.connect(database) as conn:
                conn.execute('UPDATE controller_effect_keys SET logical_json=?', (json.dumps(f.stop['command']['logical']).encode(),))
                foreign = copy.deepcopy(f.stop); foreign['identity']['sandbox'] = 'other'
                conn.execute('INSERT INTO outbox VALUES(2,?,?,?,\'dispatched\',NULL)', ('other', 'other', json.dumps(foreign).encode()))
            with self.assertRaises(ValueError):
                p._facts(p._snapshot(p.deadline))


class OriginalStopPeerTests(unittest.TestCase):
    def test_real_stop_socket_releases_only_after_original_facts_and_two_public_samples(self):
        for fault in ('none', 'peer', 'source', 'public', 'material', 'second-snapshot'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                f = fixture(Path(directory)); p = f.probe; events = []; received = []
                server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                server.bind(str(p.stop_path)); server.listen(1); server.settimeout(2)
                def serve():
                    peer, _ = server.accept()
                    with peer:
                        peer.settimeout(2); received.append(peer.recv(1))
                thread = threading.Thread(target=serve); thread.start()
                if fault == 'peer': p.host.pid += 1
                if fault == 'source': f.snapshot[3]['native']['current_vm'] += 1
                if fault == 'public': p.client.query.side_effect = TimeoutError('unknown')
                if fault == 'material': p.runtime._materials.return_value = {'changed': 'material'}
                second = copy.deepcopy(f.snapshot)
                if fault == 'second-snapshot': second[3]['global_pending_stops'].append(copy.deepcopy(f.stop))
                p._snapshot = Mock(side_effect=[f.snapshot, second])
                p._live = Mock(side_effect=lambda _: events.append('paused') or {'actual': 'paused'})
                stack.enter_context(patch.object(pending, 'private_path', side_effect=lambda value, **_: Path(value)))
                try:
                    if fault == 'none':
                        p._before_observation(p.deadline)
                        self.assertTrue(p.stop_released)
                        self.assertEqual(p.client.query.call_count, 2)
                        self.assertEqual(events, ['paused', 'paused'])
                        self.assertEqual(p.pending['original_stop'], f.stop)
                    else:
                        with self.assertRaises((ValueError, TimeoutError)):
                            p._before_observation(p.deadline)
                        self.assertFalse(p.stop_released)
                        self.assertFalse(p.failure_state()['public_completion'])
                finally:
                    p.pidfd = None; closed = p.close(); thread.join(3); server.close()
                    self.assertFalse(thread.is_alive())
                self.assertFalse(closed['physical_exit_inferred'])
                self.assertEqual(received, [b'g' if fault == 'none' else b''])


class OriginalScenarioTests(unittest.TestCase):
    def exercise(self, *, fault=False):
        events = []; clients = {}; lifetime = dict(started_at_ms=1, end_at_ms=2, max_end_at_ms=3, startup_deadline_ms=2)
        for owner in ('1', '2'):
            client = Mock(spec=scene.PublicUnloadScenario); client.owner = owner; client.session = 'session-' + owner
            client.create_acceptance = {}; client.current_acceptance = {}; client.unload_acceptance = {'key': {}}
            client.concurrent_unload_replies = [{}, {}]; client.resume_request = {}; client.witness = {}
            client.timings = [{'sent_monotonic': time.monotonic()}]; client.last_http_status = None
            client.failure_step = 'controlled'; client.timing_report.return_value = []
            client.wait_running.return_value = {'phase': 'running'}; client.wait_unloaded.return_value = {'phase': 'unloaded'}
            client.query.return_value = {'phase': 'running'}
            for name in ('create', 'unload', 'resume', 'destroy', 'wait_cleaned', 'run_public_workload', 'assert_resumed_workload'):
                getattr(client, name).side_effect = lambda *_, n=name, o=owner, **__: events.append(n + '-' + o) or {}
            clients[owner] = client
        calls = 0
        def running(session, *_, **__):
            nonlocal calls
            if session == 'session-1': calls += 1
            return dict(session=session, lifetime=lifetime,
                context={'position': 0 if session == 'session-2' or calls == 1 else 1})
        probe = Mock(spec=pending.PendingStopProbe)
        for name in ('arm', 'after_acceptance', 'finish', 'cleanup', 'close'):
            getattr(probe, name).side_effect = lambda *_, n=name, **__: events.append(n) or {}
        if fault:
            def unknown(*_):
                events.append('after_acceptance'); raise ValueError('unknown original pending Stop')
            probe.after_acceptance.side_effect = unknown
        probe.failure_state.return_value = {'original_acceptance': 'preserved', 'stop_release_sent': False}
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            runner = NS(root=Path(directory), owner_key='original', foreign_key='peer',
                services=NS(sdk_url='http://127.0.0.1:1'), measurement_identity=lambda: {}, start=lambda _: None,
                create_parameters=lambda **_: {}, record_running=Mock(side_effect=running),
                unload_evidence=Mock(return_value=dict(facts=lifetime, original_position={'position': 0}, original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation': {}}), confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}), settlement_evidence=Mock(return_value={}),
                finish=Mock(side_effect=lambda _: events.append('runtime-finish')), fail_resume_gate=Mock(return_value='unarmed'),
                close_restore_gate=Mock(), close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime, 'prepare', return_value=runner))
            stack.enter_context(patch.object(scene, 'PublicUnloadScenario', side_effect=lambda *_, owner='1': clients[owner]))
            stack.enter_context(patch.object(scene, 'check_center_configuration', return_value={'deployment': 'original'}))
            constructor = stack.enter_context(patch.object(pending, 'PendingStopProbe', return_value=probe))
            args = NS(scenario='capture-stop-pending', scenario_timeout=60, activation_gate_timeout_ms=60000)
            if fault:
                with self.assertRaisesRegex(ValueError, 'unknown original pending Stop'): scene.exercise(args)
                report = json.loads((Path(directory) / 'lifecycle-unload-report.json').read_text())
                self.assertFalse(report['complete']); self.assertEqual(len(report['known_sessions']), 2)
                self.assertFalse(report['stop_pending_failure_state']['stop_release_sent'])
                clients['1'].destroy.assert_not_called(); probe.finish.assert_not_called(); probe.cleanup.assert_not_called()
                runner.settlement_evidence.assert_not_called()
                self.assertEqual(clients['1'].resume.call_count, 1)
            else:
                report = scene.exercise(args)
                self.assertTrue(report['complete']); self.assertEqual(len(report['cycles']), 2)
                self.assertIn('original live Paused', report['acceptance_cases']['E5'])
                self.assertEqual(clients['1'].resume.call_count, 2)
                self.assertLess(events.index('resume-1'), events.index('arm'))
                self.assertLess(events.index('arm'), [i for i, value in enumerate(events) if value == 'unload-1'][1])
                self.assertLess(events.index('arm'), events.index('after_acceptance'))
                self.assertLess(events.index('after_acceptance'), events.index('finish'))
                self.assertLess(events.index('finish'), events.index('destroy-1'))
                probe.cleanup.assert_called_once(); runner.settlement_evidence.assert_called_once()
            self.assertEqual(clients['1'].unload.call_count, 2)
            constructor.assert_called_once(); probe.arm.assert_called_once(); probe.after_acceptance.assert_called_once()
            runner.finish.assert_called_once(); probe.close.assert_called_once()
            self.assertLess(events.index('runtime-finish'), events.index('close'))
            self.assertIn('stop_pending_observer_close', report)

    def test_actual_scene_uses_pre_stop_observer_then_original_recovery_and_two_restores(self):
        self.exercise()

    def test_unknown_stop_keeps_original_acceptance_without_new_unload_or_forced_destroy(self):
        self.exercise(fault=True)


if __name__ == '__main__':
    unittest.main()
