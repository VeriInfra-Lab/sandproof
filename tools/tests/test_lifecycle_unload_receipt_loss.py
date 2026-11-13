"""E13 gate/observer component tests; synthetic evidence is not KVM acceptance."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import _internal_fault_relay as relay
import lifecycle_unload_receipt_loss as recovery
from tools.tests import test_internal_fault_relay as fixtures
from tools.runtime_create_recovery import CURRENT_VERSIONS


def fixture():
    node, create, ready = fixtures.InternalFaultRelayTests().pair()
    request = copy.deepcopy(create)
    request['request']['binding']['key']['sequence'] = 2
    request['request']['binding']['expected'] = dict(state='version', version=1)
    request['request']['operation'] = dict(kind='unload', source_incarnation=1, capture=3)
    request['accepted_version'] = 2
    request['retention']['slot'] = 2
    lifetime = {k: v for k, v in ready['readiness']['facts'].items() if k != 'observed_at_ms'}
    result = dict(request=copy.deepcopy(request), outcome=dict(state='unloaded', facts=dict(
        source_incarnation=1, capture=3, manifest_digest=[7]*32, material_bytes=8192, **lifetime)))
    return node, create, ready, request, result, lifetime


class UnloadReceiptGateTests(unittest.TestCase):
    def test_dormant_once_and_pending_or_continued_are_not_completion(self):
        node, _, ready, request, result, _ = fixture()
        gate = relay.UnloadReceiptBarrier(node)
        self.assertIsNone(gate.observe(request, result))
        self.assertFalse(gate.ready.is_set())
        gate.arm()
        for outcome in (dict(state='pending'), dict(state='original_continued', facts=ready['readiness']['facts'])):
            self.assertIsNone(gate.observe(request, dict(request=request, outcome=outcome)))
            self.assertFalse(gate.ready.is_set())
        token = gate.observe(request, result)
        self.assertTrue(gate.ready.is_set())
        with self.assertRaises(ValueError):
            gate.loss_evidence()
        gate.unblock(discard=True)
        with self.assertRaises(ValueError):
            gate.loss_evidence()  # held worker has not drained
        gate.finish(token, dropped=True, failed=False)
        self.assertEqual(gate.loss_evidence()['observation'], result)
        self.assertIsNone(gate.observe(request, result))  # recovery retries pass through
        with self.assertRaises(RuntimeError):
            gate.arm()
        result['outcome']['facts']['manifest_digest'][0] = 8
        self.assertEqual(gate.loss_evidence()['observation']['outcome']['facts']['manifest_digest'], [7]*32)

    def test_every_envelope_field_and_operation_parameter_is_correlated(self):
        node, _, _, request, result, _ = fixture()
        mutations = [
            (('request', 'request', 'binding', 'protocol_version'), 3),
            (('request', 'request', 'binding', 'node', 'deployment_id'), 'elsewhere'),
            (('request', 'request', 'binding', 'node', 'node_id'), 'host-other'),
            (('request', 'request', 'binding', 'node', 'node_generation'), 2),
            (('request', 'request', 'binding', 'session'), 'sandbox-other'),
            (('request', 'request', 'binding', 'key', 'deployment'), 'other'),
            (('request', 'request', 'binding', 'key', 'owner'), '2'),
            (('request', 'request', 'binding', 'key', 'window'), 5),
            (('request', 'request', 'binding', 'key', 'sequence'), 3),
            (('request', 'request', 'binding', 'expected', 'version'), 2),
            (('request', 'accepted_version'), 3),
            (('request', 'center', 'center'), 'other'),
            (('request', 'center', 'deployment'), 'other'),
            (('request', 'retention', 'slot'), 3),
            (('request', 'retention', 'window'), 8),
            (('request', 'request', 'operation', 'source_incarnation'), 2),
            (('request', 'request', 'operation', 'capture'), 4),
            (('outcome', 'facts', 'source_incarnation'), 2),
            (('outcome', 'facts', 'capture'), 4),
            (('outcome', 'facts', 'material_bytes'), 0),
            (('outcome', 'facts', 'manifest_digest'), [7]*31),
            (('outcome', 'facts', 'started_at_ms'), True),
            (('outcome', 'facts', 'end_at_ms'), 900),
            (('outcome', 'facts', 'max_end_at_ms'), 999),
            (('outcome', 'facts', 'startup_deadline_ms'), 899),
        ]
        for path, replacement in mutations:
            gate = relay.UnloadReceiptBarrier(node)
            gate.arm()
            bad = fixtures.InternalFaultRelayTests().changed(result, path, replacement)
            with self.subTest(path=path), self.assertRaises(ValueError):
                gate.observe(request, bad)
            self.assertFalse(gate.ready.is_set())
        # Original Rust permits distinct key/retention windows (4 versus 7).
        self.assertTrue(relay.unloaded_observation(request, result, node))

    def test_missing_extra_unknown_or_untyped_fields_never_trigger_gate(self):
        node, _, _, request, result, _ = fixture()
        for container in ((), ('request',), ('outcome',), ('outcome', 'facts')):
            value = result
            for key in container:
                value = value[key]
            for field in value:
                bad = copy.deepcopy(result)
                target = bad
                for key in container:
                    target = target[key]
                del target[field]
                gate = relay.UnloadReceiptBarrier(node)
                gate.arm()
                with self.subTest(container=container, field=field), self.assertRaises(ValueError):
                    gate.observe(request, bad)
                self.assertFalse(gate.ready.is_set())
        for facts in (dict(state='stopped'), dict(state='unloaded', facts={}), dict(state='pending', facts={})):
            with self.assertRaises(ValueError):
                relay.unloaded_observation(request, dict(request=request, outcome=facts), node)
        bad = copy.deepcopy(result)
        bad['outcome']['facts']['material_bytes'] = 8192.0
        with self.assertRaises(ValueError):
            relay.unloaded_observation(request, bad, node)

    def test_concurrent_replies_must_all_drain_and_retain_identical_material(self):
        node, _, _, request, result, _ = fixture()
        gate = relay.UnloadReceiptBarrier(node)
        gate.arm()
        tokens = []
        threads = [threading.Thread(target=lambda: tokens.append(gate.observe(request, result))) for _ in range(8)]
        for worker in threads:
            worker.start()
        for worker in threads:
            worker.join(timeout=2)
            self.assertFalse(worker.is_alive())
        self.assertEqual(set(tokens), set(range(1, 9)))
        gate.unblock(discard=True)
        for token in tokens[:-1]:
            gate.finish(token, dropped=True, failed=False)
        with self.assertRaises(ValueError):
            gate.loss_evidence()
        gate.finish(tokens[-1], dropped=True, failed=False)
        self.assertEqual(gate.loss_evidence()['dropped'], 8)
        for field, replacement in (('manifest_digest', [8]*32), ('material_bytes', 8193),
                                   ('started_at_ms', 901), ('end_at_ms', 1001),
                                   ('max_end_at_ms', 1101), ('startup_deadline_ms', 1001)):
            gate = relay.UnloadReceiptBarrier(node)
            gate.arm()
            gate.observe(request, result)
            bad = copy.deepcopy(result)
            bad['outcome']['facts'][field] = replacement
            with self.subTest(field=field), self.assertRaises(ValueError):
                gate.observe(request, bad)
            gate.unblock(discard=True)
            gate.finish(1, dropped=True, failed=False)
            with self.assertRaises(ValueError):
                gate.loss_evidence()

    def handler(self, *, path=relay.UNLOAD_OBSERVATION, status=200, response=None):
        _, _, _, request, result, _ = fixture()
        return fixtures.InternalFaultRelayTests().handler(path, request, result if response is None else response, status=status)

    def test_real_handler_forwards_original_request_and_discards_before_any_reply(self):
        handler, backend = self.handler()
        gate = handler.server.relay.unload_barrier
        gate.arm()
        with patch.object(relay, '_Backend', return_value=backend), patch.object(
                gate.release, 'wait', side_effect=lambda **_: (gate.unblock(discard=True) or True)):
            handler._forward()
        handler._reply.assert_not_called()
        self.assertTrue(handler.close_connection)
        self.assertEqual(gate.loss_evidence()['dropped'], 1)
        self.assertEqual(json.loads(backend.request.call_args.kwargs['body']), fixture()[3])
        backend.close.assert_called_once()

    def test_relay_worker_error_cannot_be_laundered_as_loss(self):
        node, _, _, request, result, _ = fixture()
        value = relay.InternalFaultRelay.__new__(relay.InternalFaultRelay)
        value.unload_barrier = relay.UnloadReceiptBarrier(node)
        value.failed = threading.Event()
        gate = value.unload_barrier
        gate.arm()
        token = gate.observe(request, result)
        gate.unblock(discard=True)
        gate.finish(token, dropped=True, failed=False)
        self.assertEqual(value.unload_loss_evidence()['dropped'], 1)
        value.failed.set()
        with self.assertRaises(ValueError):
            value.unload_loss_evidence()

    def test_backend_cleanup_failure_cannot_finish_as_clean_discard(self):
        handler, backend = self.handler()
        gate = handler.server.relay.unload_barrier
        gate.arm()
        def close():
            self.assertFalse(gate.drained.is_set())
            raise OSError('controlled backend cleanup failure')
        backend.close.side_effect = close
        with patch.object(relay, '_Backend', return_value=backend), patch.object(
                gate.release, 'wait', side_effect=lambda **_: (gate.unblock(discard=True) or True)):
            with self.assertRaises(OSError):
                handler._forward()
        handler._reply.assert_not_called()
        self.assertTrue(gate.drained.is_set())
        self.assertTrue(gate.failed)
        with self.assertRaises(ValueError):
            gate.loss_evidence()

    def test_timeout_forwarding_and_worker_failure_are_not_explicit_loss(self):
        for disposition in ('timeout', 'forward', 'failure'):
            handler, backend = self.handler()
            gate = handler.server.relay.unload_barrier
            gate.arm()
            wait = (lambda **_: False) if disposition == 'timeout' else (
                lambda **_: (gate.unblock(discard=False) or True))
            if disposition == 'failure':
                handler._reply.side_effect = RuntimeError('controlled worker failure')
            with patch.object(relay, '_Backend', return_value=backend), patch.object(gate.release, 'wait', side_effect=wait):
                if disposition == 'failure':
                    with self.assertRaises(RuntimeError):
                        handler._forward()
                else:
                    handler._forward()
            self.assertFalse(gate.active)
            gate.unblock(discard=True)
            with self.assertRaises(ValueError):
                gate.loss_evidence()
            self.assertGreater(gate.response_attempts, 0)
            if disposition == 'timeout':
                handler._reply.assert_called_once_with(503, [], b'')

    def test_wrong_path_error_pending_partition_and_duplicate_json_do_not_count(self):
        for path, status, response in (
                (relay.LIFECYCLE, 200, {}), (relay.UNLOAD_OBSERVATION, 503, {}),
                (relay.UNLOAD_OBSERVATION, 200, dict(request=fixture()[3], outcome=dict(state='pending')))):
            handler, backend = self.handler(path=path, status=status, response=response)
            gate = handler.server.relay.unload_barrier
            gate.arm()
            with patch.object(relay, '_Backend', return_value=backend):
                handler._forward()
            self.assertFalse(gate.ready.is_set())
            handler._reply.assert_called_once()
        handler, backend = self.handler()
        handler.server.relay.partition.set()
        with patch.object(relay, '_Backend', return_value=backend) as factory:
            handler._forward()
        factory.assert_not_called()
        self.assertFalse(handler.server.relay.destroy_seen.is_set())
        for side in ('request', 'response'):
            handler, backend = self.handler()
            handler.server.relay.unload_barrier.arm()
            raw = b'{"request":{},"request":{}}'
            if side == 'request':
                handler.rfile = io.BytesIO(raw)
                handler.headers = fixtures.InternalFaultRelayTests().headers(f'Content-Length: {len(raw)}\r\n')
            else:
                backend.getresponse.return_value.read.return_value = raw
            with patch.object(relay, '_Backend', return_value=backend), self.assertRaises(ValueError):
                handler._forward()
            handler._reply.assert_not_called()
            self.assertFalse(handler.server.relay.unload_barrier.ready.is_set())


def audit_fixture():
    node, create, ready, request, result, lifetime = fixture()
    def operation(intent, **changes):
        value = dict(binding=intent['request']['binding'], operation=intent['request']['operation'],
            center=intent['center'], retention=intent['retention'], accepted_version=intent['accepted_version'],
            acceptance_time=dict(kind='sampled', at_ms=950), running=None,
            create_refusal=None, terminal=None, unloaded=None)
        value.update(changes)
        return copy.deepcopy(value)
    first = operation(create, operation=dict(kind='create', target_incarnation=1), running=ready['readiness']['facts'])
    first['acceptance_time']['at_ms'] = 850
    parameters = create['request']['operation']['create']
    visible_create = {k: v for k, v in parameters.items() if k != 'options'}
    visible_create.update(binding=create['request']['binding'],
        options_sha256=hashlib.sha256(bytes(parameters['options'])).hexdigest(), metadata_sha256=hashlib.sha256(b'{}').hexdigest())
    row = dict(create=visible_create, private=False, operations=[first, operation(request)],
        terminating=False, released=False, phase='unloading',
        last_running=ready['readiness']['facts'], terminal_confirmation=None)
    audit = dict(schema_version=3, versions=CURRENT_VERSIONS, deployment=node['deployment_id'], revision=3,
        sandbox_highwater=1, operation_highwater=2, reserved_resources=parameters['resources'],
        nodes=[dict(identity=node, retention_slots=2, role='runtime', mode='accepting',
                    limit=dict(instances=2, cpu=2, memory_bytes=2048, disk_bytes=8192),
                    free=parameters['resources'], templates=[], protocol_version=4,
                    healthy_until_ms=2000)], retention_lanes=[], sandboxes=[row])
    processes = {name: dict(pid=pid, start_time_ticks=100, uid=uid, gid=uid,
        executable=dict(device=10, inode=pid+100)) for name, pid, uid in (
            ('center', 101, 1001), ('redis', 102, 1002), ('host-0', 103, 0))}
    identity = {name: copy.deepcopy(first[name]) for name in ('binding', 'center', 'retention', 'accepted_version')}
    binding = request['request']['binding']
    observer = recovery.UnloadRestartObserver(node=node, acceptance=dict(session=binding['session'],
        key=binding['key'], accepted_version=request['accepted_version']), create_identity=identity, lifetime=lifetime)
    # Native JSON has independent objects, unlike aliases in a Python fixture.
    return observer, json.loads(json.dumps(audit)), processes, result


class UnloadRestartObserverTests(unittest.TestCase):
    def held(self):
        observer, audit, processes, result = audit_fixture()
        observer.hold(result, processes)
        return observer, audit, processes, result

    def stopped(self):
        observer, audit, processes, result = self.held()
        observer.center_stopped(audit, {k: v for k, v in processes.items() if k != 'center'})
        return observer, audit, processes, result

    def restarted(self):
        observer, audit, processes, result = self.stopped()
        observer.discarded(dict(observation=result, held=2, dropped=2, response_attempts=0))
        processes = copy.deepcopy(processes)
        processes['center'].update(pid=104, start_time_ticks=101)
        observer.restarted(processes)
        return observer, audit, processes, result

    def test_original_unload_recovers_in_full_without_refund_or_new_capture(self):
        observer, audit, _, result = self.restarted()
        audit['revision'] += 1
        audit['sandboxes'][0]['phase'] = 'unloaded'
        audit['sandboxes'][0]['operations'][-1]['unloaded'] = result
        evidence = observer.recovered(audit)
        self.assertEqual(observer.phase, 'recovered')
        self.assertEqual(evidence['observation'], result)
        self.assertEqual(evidence['before']['host-0'], evidence['after']['host-0'])
        evidence['observation']['outcome']['facts']['capture'] += 1
        self.assertEqual(observer.observation['outcome']['facts']['capture'], 3)

    def test_no_completion_or_changed_original_public_acceptance_lifetime(self):
        for path, replacement in ((('acceptance', 'session'), 'other'),
                (('acceptance', 'key', 'owner'), 'other'), (('acceptance', 'accepted_version'), 3),
                (('lifetime', 'started_at_ms'), 901), (('lifetime', 'end_at_ms'), 1001),
                (('lifetime', 'max_end_at_ms'), 1101), (('lifetime', 'startup_deadline_ms'), 1001)):
            observer, _, processes, result = audit_fixture()
            value = getattr(observer, path[0])
            for key in path[1:-1]:
                value = value[key]
            value[path[-1]] = replacement
            with self.subTest(path=path), self.assertRaises(ValueError):
                observer.hold(result, processes)
        observer, _, processes, result = audit_fixture()
        result['outcome'] = dict(state='pending')
        with self.assertRaises(ValueError):
            observer.hold(result, processes)

    def test_pending_must_be_unique_original_durable_unload_without_result(self):
        for mutation in ('duplicate-session', 'duplicate-key', 'changed-intent', 'already-saved', 'refunded', 'different-create'):
            observer, audit, processes, result = self.held()
            row = audit['sandboxes'][0]
            if mutation == 'duplicate-session':
                audit['sandboxes'].append(copy.deepcopy(row))
            elif mutation == 'duplicate-key':
                row['operations'][0]['binding']['key'] = row['operations'][-1]['binding']['key']
            elif mutation == 'changed-intent':
                row['operations'][-1]['retention']['window'] += 1
            elif mutation == 'already-saved':
                row['operations'][-1]['unloaded'] = result
            elif mutation == 'refunded':
                row['released'] = True
            else:
                row['operations'][0]['binding']['key']['sequence'] += 10
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                observer.center_stopped(audit, {k: v for k, v in processes.items() if k != 'center'})

    def test_survivor_process_restart_pid_reuse_credentials_or_inode_changes_fail(self):
        for role in ('redis', 'host-0'):
            for path, replacement in ((('pid',), 999), (('start_time_ticks',), 101),
                    (('uid',), 88), (('gid',), 88), (('executable', 'inode'), 999),
                    (('executable', 'device'), 11)):
                observer, audit, processes, _ = self.held()
                survivors = {k: v for k, v in processes.items() if k != 'center'}
                survivors[role] = fixtures.InternalFaultRelayTests().changed(survivors[role], path, replacement)
                with self.subTest(role=role, path=path), self.assertRaises(ValueError):
                    observer.center_stopped(audit, survivors)
                observer, _, processes, _ = self.stopped()
                observer.discarded(dict(observation=observer.observation, held=1, dropped=1, response_attempts=0))
                processes['center'].update(pid=104, start_time_ticks=101)
                processes[role] = fixtures.InternalFaultRelayTests().changed(processes[role], path, replacement)
                with self.assertRaises(ValueError):
                    observer.restarted(processes)

    def test_center_restart_requires_actual_new_identity_and_original_binary(self):
        for mutation in ('same-process', 'older-clock', 'new-binary', 'new-uid', 'bool-clock', 'shared-pid'):
            observer, _, processes, result = self.stopped()
            observer.discarded(dict(observation=result, held=1, dropped=1, response_attempts=0))
            new = processes['center']
            if mutation != 'same-process':
                new.update(pid=104, start_time_ticks=101)
            if mutation == 'older-clock': new['start_time_ticks'] = 99
            if mutation == 'new-binary': new['executable']['inode'] += 1
            if mutation == 'new-uid': new['uid'] += 1
            if mutation == 'bool-clock': new['start_time_ticks'] = True
            if mutation == 'shared-pid': new['pid'] = processes['redis']['pid']
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                observer.restarted(processes)

    def test_recovered_record_rejects_every_material_lifetime_or_envelope_omission(self):
        mutations = [(('outcome', 'facts', field), value) for field, value in (
            ('source_incarnation', 2), ('capture', 4), ('manifest_digest', [8]*32),
            ('material_bytes', 8193), ('started_at_ms', 901), ('end_at_ms', 1001),
            ('max_end_at_ms', 1101), ('startup_deadline_ms', 1001))]
        mutations += [(('request', 'retention', 'window'), 8),
                      (('request', 'center', 'center'), 'other')]
        for path, value in mutations:
            observer, audit, _, result = self.restarted()
            row = audit['sandboxes'][0]
            row['phase'] = 'unloaded'
            row['operations'][-1]['unloaded'] = fixtures.InternalFaultRelayTests().changed(result, path, value)
            with self.subTest(path=path), self.assertRaises(ValueError):
                observer.recovered(audit)
        for mutation in ('reservation', 'acceptance-time', 'history', 'metadata', 'release', 'owner'):
            observer, audit, _, result = self.restarted()
            row = audit['sandboxes'][0]
            row['phase'] = 'unloaded'
            row['operations'][-1]['unloaded'] = result
            if mutation == 'reservation': audit['reserved_resources']['cpu'] += 1
            if mutation == 'acceptance-time': row['operations'][-1]['acceptance_time']['at_ms'] += 1
            if mutation == 'history': row['operations'][0]['running']['observed_at_ms'] += 1
            if mutation == 'metadata': row['create']['metadata_sha256'] = '0'*64
            if mutation == 'release': row['released'] = True
            if mutation == 'owner': row['operations'][-1]['binding']['key']['owner'] = '2'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                observer.recovered(audit)

    def test_phase_order_forwarding_or_partial_drain_cannot_claim_recovery(self):
        observer, audit, processes, result = audit_fixture()
        with self.assertRaises(ValueError): observer.recovered(audit)
        with self.assertRaises(ValueError): observer.restarted(processes)
        with self.assertRaises(ValueError): observer.center_stopped(audit, {})
        observer.hold(result, processes)
        with self.assertRaises(ValueError): observer.restarted(processes)
        observer.center_stopped(audit, {k: v for k, v in processes.items() if k != 'center'})
        for held, dropped, attempts in ((2, 1, 0), (1, 1, 1), (0, 0, 0), (True, 1, 0)):
            with self.assertRaises(ValueError):
                observer.discarded(dict(observation=result, held=held, dropped=dropped, response_attempts=attempts))

    def test_fresh_node_health_and_physical_free_are_vetoes_not_logical_refunds(self):
        observer, audit, _, result = self.restarted()
        audit['nodes'][0]['healthy_until_ms'] += 1000
        audit['nodes'][0]['free'] = copy.deepcopy(audit['nodes'][0]['limit'])
        audit['sandboxes'][0]['phase'] = 'unloaded'
        audit['sandboxes'][0]['operations'][-1]['unloaded'] = result
        self.assertEqual(observer.recovered(audit)['observation'], result)
        for fault in ('identity', 'limit', 'templates', 'mode', 'free-beyond-limit', 'untyped-health'):
            observer, audit, _, result = self.restarted()
            node = audit['nodes'][0]
            audit['sandboxes'][0]['phase'] = 'unloaded'
            audit['sandboxes'][0]['operations'][-1]['unloaded'] = result
            if fault == 'identity': node['identity']['node_generation'] += 1
            if fault == 'limit': node['limit']['cpu'] += 1
            if fault == 'templates': node['templates'].append({'revision': 'other'})
            if fault == 'mode': node['mode'] = 'draining'
            if fault == 'free-beyond-limit': node['free']['cpu'] = node['limit']['cpu'] + 1
            if fault == 'untyped-health': node['healthy_until_ms'] = True
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                observer.recovered(audit)

    def test_survivor_sampler_uses_original_observer_without_host_stop(self):
        processes = Mock()
        processes.bindings = {name: dict(binary=Path('/fixed')/name, config=Path('/fixed')/(name+'.json'), uid=100, gid=100)
                              for name in ('center', 'redis', 'host-0')}
        processes.children = {name: Mock() for name in processes.bindings}
        processes.children['center'].poll.return_value = None
        with self.assertRaises(ValueError): recovery.observe_survivors(processes)
        processes.children['center'].poll.return_value = -9
        with patch('_center_host_services.process_facts', side_effect=lambda child, *a, **k: child) as observe:
            result = recovery.observe_survivors(processes)
        self.assertEqual(set(result), {'redis', 'host-0'})
        self.assertEqual(observe.call_count, 2)
        self.assertEqual(observe.call_args_list[0].kwargs['argv'], ['/fixed/redis', '/fixed/redis.json'])
        processes.stop.assert_not_called()
        processes.audit.assert_not_called()
        processes.start.assert_not_called()


class UnloadReceiptWiringTests(unittest.TestCase):
    def fixture(self):
        import _runtime_services
        _, pending, initial, observation = audit_fixture()
        create = copy.deepcopy(pending)
        create['sandboxes'][0]['phase'] = 'running'
        create['sandboxes'][0]['operations'] = create['sandboxes'][0]['operations'][:1]
        recovered = copy.deepcopy(pending)
        recovered['sandboxes'][0]['phase'] = 'unloaded'
        recovered['sandboxes'][0]['operations'][-1]['unloaded'] = observation
        # Actual refresh_node is allowed to refresh its physical/time veto.
        recovered['nodes'][0]['healthy_until_ms'] += 100
        recovered['nodes'][0]['free'] = copy.deepcopy(recovered['nodes'][0]['limit'])
        current = copy.deepcopy(initial)
        events = []
        processes = Mock()
        processes.children = {name: Mock() for name in initial}
        for child in processes.children.values(): child.poll.return_value = None
        processes.observe.side_effect = lambda: copy.deepcopy(current)
        gate = relay.UnloadReceiptBarrier(fixture()[0])
        def stop(name, *, force=False):
            self.assertEqual(name, 'center')
            events.append(('stop', name, force))
            processes.children[name].poll.return_value = -9
        def start(name):
            self.assertEqual(name, 'center')
            events.append(('start', name))
            current[name]['pid'] += 10
            current[name]['start_time_ticks'] += 10
            processes.children[name].poll.return_value = None
        def audit(name):
            self.assertEqual(name, 'center')
            self.assertIsNotNone(processes.children[name].poll())
            events.append(('audit', name))
            return copy.deepcopy(create if not gate.used else pending if gate.armed else recovered)
        processes.stop.side_effect = stop
        processes.start.side_effect = start
        processes.audit.side_effect = audit
        services = _runtime_services.RuntimeServices.__new__(_runtime_services.RuntimeServices)
        services.processes = processes
        services.host_configs = [dict(artifact_source=dict(download_timeout_ms=1000))]
        services.center_config = dict(deployment_id=fixture()[0]['deployment_id'], center_id='center-1')
        services.wait_ready = Mock()
        probe = recovery.UnloadReceiptRestart.__new__(recovery.UnloadReceiptRestart)
        probe.runtime = SimpleNamespace(configs={})
        probe.services = services
        probe.node = fixture()[0]
        probe.materials = Mock()
        probe.relay = relay.InternalFaultRelay.__new__(relay.InternalFaultRelay)
        probe.relay.unload_barrier = gate
        probe.relay.failed = threading.Event()
        probe.relay.stop = Mock()
        probe.started, probe.closed, probe.observer = True, False, None
        probe.deadline = time.monotonic()+5
        probe.evidence = dict(complete=False)
        probe._configs = Mock()
        running = dict(session='sandbox-1', lifetime=fixture()[5])
        public_create = dict(session='sandbox-1', key=fixture()[1]['request']['binding']['key'], accepted_version=1)
        acceptance = dict(session='sandbox-1', key=observation['request']['request']['binding']['key'], accepted_version=2)
        public = dict(session=acceptance['session'], operation=acceptance['key'],
            current_version=2, phase='unloaded', unload_outcome=observation['outcome'], create_refusal=None)
        return SimpleNamespace(probe=probe, pending=pending, current=current,
            events=events, processes=processes, gate=gate, observation=observation,
            running=running, create=public_create, acceptance=acceptance, public=public,
            deadline=probe.deadline)

    def arm(self, f):
        result = f.probe.arm(f.create, f.running, f.deadline)
        self.assertIs(result, f.probe.evidence)
        self.assertTrue(f.gate.armed)
        self.assertEqual(f.events, [('stop', 'center', False), ('audit', 'center'), ('start', 'center')])

    def interrupt(self, f):
        token = f.gate.observe(f.observation['request'], f.observation)
        def dropped():
            if f.gate.release.wait(timeout=3):
                f.gate.finish(token, dropped=True, failed=False)
        worker = threading.Thread(target=dropped)
        worker.start()
        try:
            with patch.object(recovery, 'observe_survivors', side_effect=lambda _: {
                    name: copy.deepcopy(f.current[name]) for name in ('redis', 'host-0')}):
                f.probe.interrupt(f.acceptance, f.running, f.deadline)
        finally:
            f.gate.unblock(discard=True)
            worker.join(timeout=4)
            self.assertFalse(worker.is_alive())

    def test_actual_shared_audit_gate_and_observer_orders_one_center_fault(self):
        f = self.fixture()
        self.arm(f)
        self.interrupt(f)
        self.assertEqual(f.events[3:], [('stop', 'center', True), ('audit', 'center'), ('start', 'center')])
        self.assertEqual(f.probe.observer.phase, 'restarted')
        evidence = f.probe.recovered(f.public, f.deadline)
        self.assertTrue(evidence['complete'])
        self.assertEqual(f.events[-3:], [('stop', 'center', False), ('audit', 'center'), ('start', 'center')])
        window = evidence['fault_window']
        for role in ('redis', 'host-0'):
            self.assertEqual(window['before'][role], window['after'][role])
        self.assertEqual(window['loss']['response_attempts'], 0)
        self.assertEqual(evidence['recovery']['observation'], f.observation)
        self.assertTrue(f.probe.close()['materials_removed'])
        f.probe.close()
        f.probe.relay.stop.assert_called_once_with()
        f.probe.materials.remove.assert_called_once_with()

    def test_already_saved_or_changed_pending_cannot_be_counted_as_receipt_loss(self):
        for fault in ('already-saved', 'capture', 'retention', 'reservation'):
            f = self.fixture()
            self.arm(f)
            row = f.pending['sandboxes'][0]['operations'][-1]
            if fault == 'already-saved': row['unloaded'] = f.observation
            if fault == 'capture': row['operation']['capture'] += 1
            if fault == 'retention': row['retention']['window'] += 1
            if fault == 'reservation': f.pending['sandboxes'][0]['released'] = True
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                self.interrupt(f)
            self.assertFalse(f.probe.evidence['complete'])
            self.assertEqual(len([e for e in f.events if e[0] == 'start']), 1)

    def test_expired_deadline_or_relay_error_does_not_inject_a_late_center_fault(self):
        for fault in ('deadline', 'worker-error'):
            f = self.fixture()
            self.arm(f)
            if fault == 'deadline': f.deadline = f.probe.deadline = time.monotonic()-1
            else: f.probe.relay.failed.set()
            with self.subTest(fault=fault), self.assertRaises((ValueError, TimeoutError)):
                f.probe.interrupt(f.acceptance, f.running, f.deadline)
            self.assertFalse(any(e == ('stop', 'center', True) for e in f.events))

    def test_recovery_cannot_extend_or_replace_the_original_absolute_deadline(self):
        f = self.fixture()
        self.arm(f)
        with self.assertRaisesRegex(ValueError, 'absolute deadline'):
            f.probe.interrupt(f.acceptance, f.running, f.deadline+1)
        self.assertFalse(any(e == ('stop', 'center', True) for e in f.events))

    def test_public_outcome_changes_reject_before_another_native_audit(self):
        for fault in ('session', 'owner', 'version', 'capture', 'material', 'lifetime'):
            f = self.fixture()
            self.arm(f)
            self.interrupt(f)
            public = copy.deepcopy(f.public)
            if fault == 'session': public['session'] = 'other'
            if fault == 'owner': public['operation']['owner'] = '2'
            if fault == 'version': public['current_version'] = 3
            if fault == 'capture': public['unload_outcome']['facts']['capture'] += 1
            if fault == 'material': public['unload_outcome']['facts']['material_bytes'] += 1
            if fault == 'lifetime': public['unload_outcome']['facts']['end_at_ms'] += 1
            before = list(f.events)
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                f.probe.recovered(public, f.deadline)
            self.assertEqual(f.events, before)
            self.assertFalse(f.probe.evidence['complete'])

    def test_shutdown_error_after_recovery_marks_failure_and_preserves_material(self):
        f = self.fixture()
        self.arm(f)
        self.interrupt(f)
        f.probe.recovered(f.public, f.deadline)
        f.probe.relay.stop.side_effect = f.probe.relay.failed.set
        for _ in range(2):
            with self.assertRaises(ValueError): f.probe.close()
        self.assertFalse(f.probe.evidence['complete'])
        f.probe.materials.remove.assert_not_called()

    def test_install_seals_the_fresh_endpoint_and_does_not_start_any_role(self):
        import _runtime_services
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root/'center.json'
            path.write_text('{}')
            path.chmod(0o600)
            inode = path.stat().st_ino
            services = _runtime_services.RuntimeServices.__new__(_runtime_services.RuntimeServices)
            services.processes = Mock()
            services.processes.children = {}
            services.processes.bindings = {'center': {'config': path}}
            services.host_configs = [dict(inbox=dict(node=fixture()[0]), internal=dict(bind='127.0.0.1:2345'))]
            services.center_config = dict(nodes=[dict(address='127.0.0.1:2345', registration=dict(identity=fixture()[0]))])
            services.layout = dict(host_controls=[root])
            services.native_pools = [dict(bounded_host=dict(physical_storage=dict(service_data=[
                dict(root=str(root), owner_uid=0, peak_bytes=relay.MATERIAL_BYTES, inode_budget=8)])))]
            services.tls_receipt = {'original': 'receipt'}
            runtime = SimpleNamespace(services=services, args=SimpleNamespace(tls_material=root/'operator'))
            with patch.object(recovery, 'RelayMaterials') as materials, \
                    patch.object(recovery, 'InternalFaultRelay') as factory, \
                    patch('server_process_crash.free_loopback_port', return_value=3456):
                probe = recovery.UnloadReceiptRestart(runtime)
                self.assertEqual(path.stat().st_ino, inode)
                self.assertEqual(json.loads(path.read_text())['nodes'][0]['address'], '127.0.0.1:3456')
                self.assertEqual(factory.call_args.kwargs['backend'], ('127.0.0.1', 2345))
                materials.assert_called_once_with(runtime.args.tls_material, root, services.tls_receipt)
                services.processes.initialize.assert_not_called()
                services.processes.start.assert_not_called()
                factory.return_value.start.assert_not_called()
                factory.return_value.failed.is_set.return_value = False
                probe.close()
                factory.return_value.server.server_close.assert_called_once_with()
                factory.return_value.stop.assert_not_called()

    def test_missing_declared_budget_fails_before_key_copy_or_config_mutation(self):
        services = SimpleNamespace(processes=SimpleNamespace(children={}),
            host_configs=[dict(inbox=dict(node=fixture()[0]))],
            center_config=dict(nodes=[dict(registration=dict(identity=fixture()[0]))]),
            layout=dict(host_controls=[Path('/original/control')]),
            native_pools=[dict(bounded_host=dict(physical_storage=dict(service_data=[])))])
        runtime = SimpleNamespace(services=services)
        with patch.object(recovery, 'RelayMaterials') as materials, self.assertRaises(ValueError):
            recovery.UnloadReceiptRestart(runtime)
        materials.assert_not_called()

    def test_original_runtime_starts_the_relay_before_the_product_cohort(self):
        import lifecycle_unload_runtime
        state = lifecycle_unload_runtime.UnloadRuntime.__new__(lifecycle_unload_runtime.UnloadRuntime)
        state.services, state.receipt_restart = Mock(), Mock()
        events = []
        state.receipt_restart.start.side_effect = lambda _: events.append('relay')
        state.services.start.side_effect = lambda: events.append('products')
        deadline = time.monotonic()+5
        state.start(deadline)
        self.assertEqual(events, ['relay', 'products'])
        state.receipt_restart.start.assert_called_once_with(deadline)
        self.assertEqual(state.deadline, deadline)
        self.assertTrue(state.services.wait_ready.call_args.kwargs['for_admission'])

    def test_selected_public_entry_reaches_fault_then_native_recovery_before_host_audit(self):
        import lifecycle_unload_scenario as scene
        f = self.fixture()
        events = []
        def action(name, value=None, *, fail=False):
            def invoke(*args, **kwargs):
                events.append(name)
                if fail: raise RuntimeError('controlled end of partial orchestration check')
                return value
            return Mock(side_effect=invoke)
        for selector in ('unload-receipt-restart', 'normal'):
            events.clear()
            with self.subTest(selector=selector), tempfile.TemporaryDirectory() as directory:
                relay_probe = SimpleNamespace(arm=action('arm', {}), interrupt=action('interrupt'),
                    recovered=action('recovered', fail=True), close=action('close-relay', {}))
                runner = SimpleNamespace(root=Path(directory),
                    services=SimpleNamespace(sdk_url='http://127.0.0.1:3456'),
                    owner_key='controlled', foreign_key='foreign', receipt_restart=relay_probe,
                    measurement_identity=lambda:{}, start=action('start'), create_parameters=lambda:{},
                    record_running=action('running-evidence', f.running),
                    unload_evidence=action('host-audit', fail=True),
                    finish=action('finish'), fail_resume_gate=lambda:None,
                    close_restore_gate=lambda:None, close_capture_cancel_gate=lambda:None)
                client = SimpleNamespace(owner='1', session='sandbox-1', create_acceptance=f.create,
                    current_acceptance=f.acceptance, unload_acceptance=f.acceptance,
                    concurrent_unload_replies=[f.acceptance, f.acceptance], timings=[],
                    failure_step='controlled', last_http_status=None, timing_report=lambda:[],
                    create=action('create', f.create), wait_running=action('running', {}),
                    assert_owner_isolation=lambda:None, run_public_workload=lambda _: {},
                    step=lambda _:None, operation_evidence_completed=lambda:None,
                    unload=action('unload', f.acceptance), assert_data_closed=action('data-closed'),
                    wait_unloaded=action('public-unloaded', f.public))
                args = SimpleNamespace(scenario=selector, scenario_timeout=5, activation_gate_timeout_ms=60000)
                with patch.dict(sys.modules, {'lifecycle_unload_runtime': SimpleNamespace(prepare=lambda _:runner)}), \
                        patch.object(scene, 'check_center_configuration', return_value={'deployment': 'exact-test'}), \
                        patch.object(scene, 'PublicUnloadScenario', return_value=client), self.assertRaises(RuntimeError):
                    scene.exercise(args)
                report = json.loads((Path(directory)/'lifecycle-unload-report.json').read_text())
                self.assertFalse(report['complete'])
                if selector == 'unload-receipt-restart':
                    self.assertEqual(events, ['start', 'create', 'running', 'running-evidence', 'arm',
                        'unload', 'data-closed', 'interrupt', 'public-unloaded', 'recovered', 'finish', 'close-relay'])
                    runner.unload_evidence.assert_not_called()
                    relay_probe.interrupt.assert_called_once_with(f.acceptance, f.running,
                        relay_probe.arm.call_args.args[-1])
                    self.assertEqual(report['phase'], 'unload-receipt-recovered-native-center')
                else:
                    relay_probe.arm.assert_not_called()
                    relay_probe.interrupt.assert_not_called()
                    relay_probe.close.assert_not_called()
                    self.assertIn('host-audit', events)


if __name__ == '__main__':
    unittest.main()
