"""Current Create observation/partition correlation; no TLS or KVM claim."""
import copy
from http.client import parse_headers
import io
import json
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT/'tests/system'))
import _internal_fault_relay as relay


class InternalFaultRelayTests(unittest.TestCase):
    def headers(self, value):
        return parse_headers(io.BytesIO(value.encode('ascii') + b'\r\n'))

    def pair(self):
        # sandbox-protocol lifecycle::AcceptedLifecycleIntent/CreateObservation;
        # complete serde fields, no legacy ControlIdentity or host sequence.
        node = dict(deployment_id='exact-test', node_id='host-runtime', node_generation=1)
        binding = dict(protocol_version=3, node=node, session='sandbox-1',
            key=dict(deployment='exact-test', owner='1', window=4, sequence=1),
            expected={'state': 'absent'})
        create = dict(template_revision='template-1', template_digest=[1]*32,
            resources=dict(instances=1, cpu=1, memory_bytes=1024, disk_bytes=4096),
            options=list(b'{}'), startup_deadline_ms=1000, timeout_ms=100, max_lifetime_ms=200)
        request = dict(request=dict(binding=binding,
            operation=dict(kind='create', create=create, target_incarnation=1)),
            accepted_version=1, center=dict(deployment='exact-test', center='center-1'),
            retention=dict(slot=1, window=7))
        response = dict(request=copy.deepcopy(request), current_request=copy.deepcopy(request),
            historical_create_succeeded=True, readiness=dict(state='observed', facts=dict(
                started_at_ms=900, end_at_ms=1000, max_end_at_ms=1100,
                startup_deadline_ms=1000, observed_at_ms=950)))
        return node, request, response

    def changed(self, value, path, replacement):
        result = copy.deepcopy(value)
        target = result
        for key in path[:-1]:
            target = target[key]
        target[path[-1]] = replacement
        return result

    def test_paths_have_only_fixed_original_internal_destinations(self):
        # Match the current Rust server constants, not whatever the relay lists.
        expected = (
            '/internal/lifecycle', '/internal/lifecycle/result-confirmation',
            '/internal/lifecycle/window-close', '/internal/lifecycle/create-observation',
            '/internal/lifecycle/terminal-observation', '/internal/lifecycle/renewal-confirmation',
            '/internal/node-status', '/internal/probe', '/internal/template-cache',
            '/internal/template-validation-evidence',
        )
        self.assertEqual(relay.PATHS, frozenset(expected))
        for path in (*expected, '/internal/data/process.Process/Start',
                     '/internal/data/files?path=%2Ftmp%2Fx'):
            self.assertTrue(relay.allowed_path(path), path)
        for path in ('https://other.test/internal/lifecycle', '//other.test/path',
                     '/v2/sandboxes', '/internal/lifecycle?target=other', '/internal/shell',
                     '/internal/v1/lifecycle', '/internal/v2/node-status',
                     '/internal/v2/lifecycle', '/internal/v2/probe',
                     '/internal/v2/data/process.Process/Start'):
            self.assertFalse(relay.allowed_path(path), path)

    def test_finite_length_and_chunked_bodies_keep_original_bytes(self):
        self.assertEqual(relay.read_body(io.BytesIO(b'body'),self.headers('Content-Length: 4\r\n')), b'body')
        self.assertEqual(relay.read_body(io.BytesIO(b'2\r\nbo\r\n2\r\ndy\r\n0\r\n\r\n'),
            self.headers('Transfer-Encoding: chunked\r\n')), b'body')
        self.assertEqual(relay.read_body(io.BytesIO(), self.headers('')), b'')

    def test_ambiguous_negative_oversized_or_truncated_framing_refuses(self):
        fixtures = [
            ('Content-Length: 1\r\nContent-Length: 1\r\n', b'x'),
            ('Content-Length: 1\r\nTransfer-Encoding: chunked\r\n', b'x'),
            ('Content-Length: -1\r\n', b''), ('Content-Length: 5\r\n', b'body'),
            ('Content-Length: 9\r\n', b'ninebytes'), ('Transfer-Encoding: gzip\r\n', b''),
            ('Transfer-Encoding: chunked\r\n', b'9\r\nninebytes\r\n0\r\n\r\n'),
            ('Transfer-Encoding: chunked\r\n', b'1\r\nx\r\n0\r\nX-Trailer: no\r\n\r\n'),
            ('Transfer-Encoding: chunked\r\n', b'1;ext\r\nx\r\n0\r\n\r\n'),
            ('Transfer-Encoding: chunked\r\n', b'2\r\nx\r\n'),
        ]
        for headers, body in fixtures:
            with self.subTest(headers=headers, body=body), self.assertRaises(ValueError):
                relay.read_body(io.BytesIO(body),self.headers(headers),limit=8)

    def test_only_exact_real_ready_sets_barrier_before_forwarding(self):
        node, request, response = self.pair()
        barrier = relay.ReceiptBarrier(node)
        barrier.arm()
        for state in ('pending', 'not_current', 'not_running', 'expired', 'refused'):
            readiness = {'state': state}
            if state == 'refused':
                readiness['reason'] = 'capacity'
            self.assertFalse(barrier.observe(request, dict(response, readiness=readiness)))
            self.assertFalse(barrier.ready.is_set())
        self.assertTrue(barrier.observe(request, response))
        self.assertTrue(barrier.ready.is_set())
        self.assertFalse(barrier.release.is_set())
        self.assertEqual(barrier.forwarded, 0)
        self.assertEqual(barrier.captured, response)
        barrier.unblock(discard=True)
        self.assertTrue(barrier.release.is_set())
        self.assertTrue(barrier.discard)
        self.assertEqual(barrier.forwarded, 0)

    def test_observed_requires_full_original_current_and_typed_lifetime(self):
        node, request, response = self.pair()
        mutations = [
            (('request', 'request', 'binding', 'session'), 'sandbox-2'),
            (('request', 'request', 'operation', 'create', 'options'), list(b'{ }')),
            (('current_request', 'request', 'binding', 'key', 'owner'), '2'),
            (('current_request', 'request', 'binding', 'key', 'window'), 5),
            (('current_request', 'request', 'binding', 'key', 'sequence'), 2),
            (('current_request', 'request', 'operation', 'target_incarnation'), 2),
            (('current_request', 'center', 'center'), 'other-center'),
            (('current_request', 'retention', 'window'), 8),
            (('current_request', 'retention', 'slot'), 2),
            (('current_request', 'accepted_version'), 2),
            (('historical_create_succeeded',), False),
            (('historical_create_succeeded',), 1),
            (('readiness', 'facts', 'end_at_ms'), 1001),
            (('readiness', 'facts', 'max_end_at_ms'), 1101),
            (('readiness', 'facts', 'startup_deadline_ms'), 1001),
            (('readiness', 'facts', 'started_at_ms'), 1001),
            (('readiness', 'facts', 'observed_at_ms'), 899),
            (('readiness', 'facts', 'observed_at_ms'), 1000),
            (('readiness', 'facts', 'observed_at_ms'), True),
            (('readiness', 'facts', 'observed_at_ms'), 2**64),
            (('readiness', 'facts', 'observed_at_ms'), 950.0),
        ]
        for path, value in mutations:
            barrier = relay.ReceiptBarrier(node)
            barrier.arm()
            with self.subTest(path=path, value=value), self.assertRaises(ValueError):
                barrier.observe(request, self.changed(response, path, value))
            self.assertFalse(barrier.ready.is_set())
        altered = copy.deepcopy(request)
        altered['request']['binding']['node']['node_generation'] = 2
        with self.assertRaises(ValueError):
            barrier.observe(altered, dict(response, request=altered, current_request=altered))

    def test_legacy_record_unknown_missing_and_bool_identity_never_become_ready(self):
        node, request, response = self.pair()
        invalid = [dict(identity={}, fact={'kind': 'ready'}, observation_version=2),
                   dict(intent=request, current_session_version=1, current=True, terminal=False),
                   dict(response, extra=True), dict(response, readiness={'state': 'observed'}),
                   self.changed(response, ('readiness', 'facts', 'extra'), 1),
                   self.changed(response, ('current_request', 'request', 'binding', 'node', 'node_generation'), True)]
        for altered in invalid:
            barrier = relay.ReceiptBarrier(node)
            barrier.arm()
            with self.subTest(response=altered), self.assertRaises(ValueError):
                barrier.observe(request, altered)
            self.assertFalse(barrier.ready.is_set())
        with self.assertRaises(ValueError):
            barrier.observe(dict(identity={}, intent={'kind': 'create'}, create={}), response)

    def test_start_at_deadline_matches_current_rust_boundary(self):
        node, request, response = self.pair()
        response['readiness']['facts'].update(started_at_ms=1000, end_at_ms=1100,
            max_end_at_ms=1200, observed_at_ms=1000)
        barrier = relay.ReceiptBarrier(node)
        barrier.arm()
        self.assertTrue(barrier.observe(request, response))

    def test_parallel_repeated_ready_can_advance_observation_not_execution(self):
        node, request, response = self.pair()
        barrier = relay.ReceiptBarrier(node)
        barrier.arm()
        observed = []
        workers = [threading.Thread(target=lambda: observed.append(barrier.observe(request, response)))
                   for _ in range(4)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(timeout=2)
            self.assertFalse(worker.is_alive())
        self.assertEqual(observed, [True]*4)
        later = self.changed(response, ('readiness', 'facts', 'observed_at_ms'), 960)
        self.assertTrue(barrier.observe(request, later))
        changed = copy.deepcopy(response)
        changed['readiness']['facts'].update(started_at_ms=901, end_at_ms=1001, max_end_at_ms=1101)
        with self.assertRaises(ValueError):
            barrier.observe(request, changed)
        response['readiness']['facts']['end_at_ms'] = 999
        self.assertEqual(barrier.captured['readiness']['facts']['end_at_ms'], 1000)
        barrier.unblock(discard=True)

    def partition_relay(self, node):
        value = relay.InternalFaultRelay.__new__(relay.InternalFaultRelay)
        value.barrier = relay.ReceiptBarrier(node)
        value.partition_requests = []
        value.lock = threading.Lock()
        value.destroy_seen = threading.Event()
        return value

    def destroy(self, request):
        result = copy.deepcopy(request)
        result['request']['binding']['expected'] = {'state': 'version', 'version': 1}
        result['request']['binding']['key']['sequence'] = 2
        result['request']['operation'] = {'kind': 'destroy'}
        result['accepted_version'] = 2
        return result

    def test_partition_retains_complete_requests_and_destroy_requires_record_path(self):
        node, request, _ = self.pair()
        value = self.partition_relay(node)
        destroy = self.destroy(request)
        value.note_partition(request, path=relay.CREATE_OBSERVATION)
        value.note_partition(request, path=relay.CREATE_OBSERVATION)
        self.assertEqual(value.partition_requests, [request])
        self.assertFalse(value.destroy_seen.is_set())
        value.note_partition(destroy, path=relay.CREATE_OBSERVATION)
        self.assertFalse(value.destroy_seen.is_set())
        value.note_partition(destroy, path=relay.LIFECYCLE)
        self.assertTrue(value.destroy_seen.is_set())
        self.assertEqual(value.partition_requests, [request, destroy])
        destroy['retention']['slot'] = 2
        self.assertEqual(value.partition_requests[1]['retention']['slot'], 1)

    def test_partition_foreign_legacy_and_exhausted_evidence_refuse(self):
        node, request, _ = self.pair()
        value = self.partition_relay(node)
        for bad in (dict(identity={}, intent={'kind': 'destroy'}),
                    self.changed(request, ('request', 'binding', 'node', 'node_id'), 'other')):
            with self.assertRaises(ValueError):
                value.note_partition(bad, path=relay.LIFECYCLE)
        for sequence in range(1, 33):
            value.note_partition(self.changed(request, ('request', 'binding', 'key', 'sequence'), sequence),
                path=relay.LIFECYCLE)
        value.note_partition(request, path=relay.LIFECYCLE)
        with self.assertRaises(ValueError):
            value.note_partition(self.changed(request, ('request', 'binding', 'key', 'sequence'), 33),
                path=relay.LIFECYCLE)
        self.assertEqual(len(value.partition_requests), 32)
        self.assertFalse(value.destroy_seen.is_set())

    def handler(self, path, request, response, *, status=200):
        node, _, _ = self.pair()
        body = json.dumps(request).encode()
        value = relay._Handler.__new__(relay._Handler)
        value.path, value.command = path, 'POST'
        value.headers = self.headers(f'Content-Length: {len(body)}\r\n')
        value.rfile = io.BytesIO(body)
        value.server = Mock()
        value.server.relay = self.partition_relay(node)
        value.server.relay.partition = threading.Event()
        value.server.relay.backend = ('127.0.0.1', 2345)
        value.server.relay.materials = None
        value._reply = Mock()
        backend = Mock()
        backend.getresponse.return_value.status = status
        backend.getresponse.return_value.read.return_value = json.dumps(response).encode()
        backend.getresponse.return_value.getheaders.return_value = []
        return value, backend

    def test_forward_captures_only_actual_create_observation_and_discards_all_bytes(self):
        _, request, response = self.pair()
        value, backend = self.handler(relay.CREATE_OBSERVATION, request, response)
        barrier = value.server.relay.barrier
        barrier.arm()
        barrier.discard = True
        with patch.object(relay, '_Backend', return_value=backend), patch.object(barrier.release, 'wait', return_value=True):
            value._forward()
        self.assertEqual(barrier.captured, response)
        self.assertEqual(barrier.forwarded, 0)
        self.assertTrue(value.close_connection)
        value._reply.assert_not_called()
        backend.close.assert_called_once()
        self.assertEqual(json.loads(backend.request.call_args.kwargs['body']), request)

    def test_record_or_failed_observation_never_sets_ready(self):
        _, request, response = self.pair()
        record = dict(intent=request, current_session_version=1, current=True, terminal=False)
        for path, result, status in ((relay.LIFECYCLE, record, 200),
                                     (relay.CREATE_OBSERVATION, response, 503)):
            value, backend = self.handler(path, request, result, status=status)
            value.server.relay.barrier.arm()
            with patch.object(relay, '_Backend', return_value=backend):
                value._forward()
            self.assertFalse(value.server.relay.barrier.ready.is_set())
            value._reply.assert_called_once()
            self.assertEqual(value._reply.call_args.args[0], status)

    def test_partition_records_current_body_without_contacting_backend(self):
        _, request, _ = self.pair()
        value, backend = self.handler(relay.LIFECYCLE, self.destroy(request), {})
        value.server.relay.partition.set()
        with patch.object(relay, '_Backend', return_value=backend) as factory:
            value._forward()
        factory.assert_not_called()
        value._reply.assert_called_once_with(503, [], b'')
        self.assertTrue(value.server.relay.destroy_seen.is_set())
        self.assertEqual(value.server.relay.partition_requests, [self.destroy(request)])

    def test_duplicate_request_or_response_json_refuses_without_synthetic_reply(self):
        _, request, response = self.pair()
        for side in ('request', 'response'):
            value, backend = self.handler(relay.CREATE_OBSERVATION, request, response)
            value.server.relay.barrier.arm()
            if side == 'request':
                raw = b'{"accepted_version":1,"accepted_version":1}'
                value.rfile = io.BytesIO(raw)
                value.headers = self.headers(f'Content-Length: {len(raw)}\r\n')
            else:
                backend.getresponse.return_value.read.return_value = b'{"request":{},"request":{}}'
            with patch.object(relay, '_Backend', return_value=backend), self.assertRaises(ValueError):
                value._forward()
            value._reply.assert_not_called()
            self.assertFalse(value.server.relay.barrier.ready.is_set())

    def test_no_unbounded_target_or_recursive_relay_is_constructed(self):
        for listen, backend in ((('0.0.0.0',1234),('127.0.0.1',2345)),
                                (('127.0.0.1',1234),('other.test',2345)),
                                (('127.0.0.1',1234),('127.0.0.1',1234))):
            with self.assertRaises(ValueError):
                relay.InternalFaultRelay(listen=listen,backend=backend,node={},materials=None)

    def test_original_host_transport_loss_is_unavailable_not_a_synthetic_fact(self):
        value=relay._Handler.__new__(relay._Handler)
        value.path=relay.STATUS_PATH
        value.command='POST'
        value.headers=self.headers('Content-Length: 0\r\n')
        value.rfile=io.BytesIO()
        value.server=Mock()
        value.server.relay.partition.is_set.return_value=False
        value._reply=Mock()
        backend=Mock()
        backend.request.side_effect=relay.http.client.RemoteDisconnected()
        with patch.object(relay,'_Backend',return_value=backend):
            value._forward()
        value._reply.assert_called_once_with(503,[],b'')
        backend.close.assert_called_once()
        value.server.relay.failed.set.assert_not_called()

    def test_backend_requires_http_alpn_and_closes_wrapped_socket_on_rejection(self):
        materials = Mock()
        connection = materials.client.wrap_socket.return_value
        for selected in (None, 'h2'):
            with self.subTest(selected=selected):
                connection.reset_mock()
                connection.selected_alpn_protocol.return_value = selected
                backend = relay._Backend(('127.0.0.1', 2345), materials)
                raw = Mock()
                with patch.object(relay.socket, 'create_connection', return_value=raw), self.assertRaises(ValueError):
                    backend.connect()
                connection.close.assert_called_once()
                raw.close.assert_called_once()
                connection.getpeercert.assert_not_called()

    def test_frontend_requires_http_alpn_and_closes_wrapped_socket_on_rejection(self):
        server = relay._FiniteServer.__new__(relay._FiniteServer)
        server.relay = Mock()
        connection = server.relay.materials.server.wrap_socket.return_value
        for selected in (None, 'h2'):
            with self.subTest(selected=selected):
                connection.reset_mock()
                connection.selected_alpn_protocol.return_value = selected
                raw = Mock()
                with patch.object(relay.HTTPServer, 'get_request', return_value=(raw, ('127.0.0.1', 1234))), self.assertRaises(OSError):
                    server.get_request()
                connection.close.assert_called_once()
                raw.close.assert_called_once()
                connection.getpeercert.assert_not_called()

    def test_bad_authenticated_backend_data_still_fails_instead_of_becoming_unavailable(self):
        value=relay._Handler.__new__(relay._Handler)
        value.path=relay.STATUS_PATH
        value.command='POST'
        value.headers=self.headers('Content-Length: 0\r\n')
        value.rfile=io.BytesIO()
        value.server=Mock()
        value.server.relay.partition.is_set.return_value=False
        value._reply=Mock()
        backend=Mock()
        backend.getresponse.return_value.read.return_value=b'x'*(relay.BODY_BYTES+1)
        with patch.object(relay,'_Backend',return_value=backend),self.assertRaises(ValueError):
            value._forward()
        value._reply.assert_not_called()
        backend.close.assert_called_once()


if __name__ == '__main__':
    unittest.main()
