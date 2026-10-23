"""Current fixed paths/framing and historical barrier vectors; no TLS or KVM.

The barrier packet vectors still need migration to current Create observations;
these pure tests do not certify the current lifecycle fault matrix.
"""
import copy
from http.client import parse_headers
import io
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
        node = dict(deployment_id='exact-test', node_id='host-runtime', node_generation=1)
        identity = dict(protocol_version=2, node=node, tenant_id='1', sandbox_id='sandbox-1',
                        operation_id='operation-1', request_window=1, retention_slot=1, intent_version=1)
        request = dict(identity=identity, intent={'kind':'create'}, create=dict(
            startup_deadline_ms=1000, timeout_ms=100, max_lifetime_ms=200))
        response = dict(identity=copy.deepcopy(identity), observation_version=2,
            fact=dict(kind='ready', started_at_ms=900, end_at_ms=1000, max_end_at_ms=1100))
        return node, request, response

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
        self.assertFalse(barrier.observe(request, dict(response, fact={'kind':'recorded'})))
        self.assertFalse(barrier.ready.is_set())
        self.assertTrue(barrier.observe(request,response))
        self.assertTrue(barrier.ready.is_set())
        self.assertFalse(barrier.release.is_set())
        self.assertEqual(barrier.forwarded, 0)
        self.assertEqual(barrier.captured['identity'], request['identity'])
        barrier.unblock(discard=True)
        self.assertTrue(barrier.release.is_set())
        self.assertTrue(barrier.discard)
        self.assertEqual(barrier.forwarded, 0)

    def test_foreign_node_identity_and_restarted_lifetime_are_never_ready(self):
        node, request, response = self.pair()
        mutations = [('identity','sandbox_id','sandbox-2'), ('fact','end_at_ms',1001),
                     ('fact','max_end_at_ms',1101), ('fact','started_at_ms',1000)]
        for section, key, value in mutations:
            barrier = relay.ReceiptBarrier(node)
            barrier.arm()
            altered = copy.deepcopy(response)
            altered[section][key] = value
            with self.subTest(section=section,key=key), self.assertRaises(ValueError):
                barrier.observe(request,altered)
            self.assertFalse(barrier.ready.is_set())
        request['identity']['node'] = dict(node,node_generation=2)
        response['identity'] = copy.deepcopy(request['identity'])
        with self.assertRaises(ValueError):
            barrier.observe(request,response)

    def test_parallel_repeated_ready_cannot_change_capture(self):
        node, request, response = self.pair()
        barrier = relay.ReceiptBarrier(node)
        barrier.arm()
        observed = []
        workers = [threading.Thread(target=lambda:observed.append(barrier.observe(request,response)))
                   for _ in range(4)]
        for worker in workers:
            worker.start()
        for worker in workers:
            worker.join(timeout=2)
            self.assertFalse(worker.is_alive())
        self.assertEqual(observed,[True]*4)
        later = copy.deepcopy(response)
        later['observation_version'] = 3
        with self.assertRaises(ValueError):
            barrier.observe(request,later)
        barrier.unblock(discard=True)

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
