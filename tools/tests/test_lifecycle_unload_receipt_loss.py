"""E13 gate/observer component tests; synthetic evidence is not KVM acceptance."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import _internal_fault_relay as relay
import lifecycle_unload_receipt_loss as recovery
from tools.tests import test_internal_fault_relay as fixtures


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
    audit = dict(schema_version=3, versions={}, deployment=node['deployment_id'], revision=3,
        sandbox_highwater=1, operation_highwater=2, reserved_resources=parameters['resources'],
        nodes=[], retention_lanes=[], sandboxes=[row])
    processes = {name: dict(pid=pid, start_time_ticks=100, uid=uid, gid=uid,
        executable=dict(device=10, inode=pid+100)) for name, pid, uid in (
            ('center', 101, 1001), ('redis', 102, 1002), ('host-0', 103, 0))}
    identity = {name: copy.deepcopy(first[name]) for name in ('binding', 'center', 'retention', 'accepted_version')}
    binding = request['request']['binding']
    observer = recovery.UnloadRestartObserver(node=node, acceptance=dict(session=binding['session'],
        key=binding['key'], accepted_version=request['accepted_version']), create_identity=identity, lifetime=lifetime)
    return observer, copy.deepcopy(audit), processes, result


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


if __name__ == '__main__':
    unittest.main()
