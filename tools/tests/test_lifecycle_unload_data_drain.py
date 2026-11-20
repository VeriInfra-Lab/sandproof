"""Current rendered host inputs for the existing drain oracle; no VM or SDK."""
import copy
from contextlib import contextmanager
import json
import os
from pathlib import Path
import queue
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import _center_host_services as roles
import lifecycle_unload_data_drain as drain
import lifecycle_unload_runtime as runtime
from tools.template_build_config import host_uid_range


def rendered_native(state_directory):
    # The original Rust deployment tests deserialize/validate these samples.
    # Feed their current raw native/capture shapes through the same role render
    # used by the real lifecycle scenario, rather than inventing a NodeConfig.
    host = json.loads((ROOT / 'deploy/host.example.json').read_text())
    center = json.loads((ROOT / 'deploy/center.example.json').read_text())
    native = host['native']
    native['node_state_dir'] = str(state_directory)
    _, hosts = roles.render([native],
        registered_catalogs=[native['templates']],
        capture_settings=[host['capture']],
        deployment=host['inbox']['node']['deployment_id'],
        center_root='/controlled/center', host_tls_roots=['/controlled/host/tls'],
        host_addresses=['127.0.0.1:9001'], public_address='127.0.0.1:9000',
        artifact_address='127.0.0.1:9002', redis_address='127.0.0.1:9003',
        password='TESTONLYREDISKEY', key='TESTONLYPUBLICKEY',
        retained_sessions=host['inbox']['max_sessions'],
        registry=center['template_registry'],
        versions={'internal_protocol': center['nodes'][0]['registration']['protocol_version']})
    return hosts[0]['native']


class DataDrainRouteSchemaTests(unittest.TestCase):
    def test_formal_rendered_native_scans_routes_without_derived_input_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            routes = root / 'helper-gates'
            routes.mkdir(mode=0o700)
            native = rendered_native(root)
            self.assertEqual(set(native['bounded_host']), {
                'console_policy', 'maintenance', 'physical_storage', 'uid_base', 'uplink_base'})
            self.assertGreater(len(host_uid_range(native)), 0)
            before = copy.deepcopy(native)
            running = {'context': {'position': 0, 'binding': {'vm': 7}}}
            # Only root ownership is controlled here. No marker, process, data
            # request or cleanup is manufactured by this empty-route check.
            with patch.object(drain, '_private_directory', return_value=routes):
                self.assertEqual(drain._live_routes(native, running), {})
            self.assertEqual(native, before)

    @contextmanager
    def route_cell(self, maximum=5):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            directory = root / 'helper-gates'
            directory.mkdir(mode=0o700)
            native = rendered_native(root)
            native['data_transfer']['max_transfers_per_vm'] = maximum
            path = directory / '0.lock'
            path.touch(mode=0o600)
            actual = path.stat()
            # Match the original RuntimeHostSlot/InflightRecord serialization.
            # Only private-root ownership is supplied as a controlled fact;
            # the process below is this real test's current Linux process.
            context = dict(position=0, vm=7,
                uplink_index=native['bounded_host']['uplink_base'],
                helper_gate=dict(path=str(path), device=actual.st_dev, inode=actual.st_ino,
                    max_data_routes_per_slot=maximum))
            fields = Path(f'/proc/{os.getpid()}/stat').read_text().rsplit(')', 1)[1].split()
            process = dict(pid=os.getpid(), start_time_ticks=int(fields[19]),
                boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            record = dict(version=1, context=context, process=process,
                command_digest=[1]*32, route_index=maximum-1, control=False)
            marker = directory / f'0.lock.route-{maximum-1}.json'
            marker.write_text(json.dumps(record))
            metadata = SimpleNamespace(st_dev=actual.st_dev, st_ino=actual.st_ino,
                st_mode=stat.S_IFREG | 0o600, st_uid=0, st_nlink=1)
            def read_marker(value, _limit):
                return json.loads(Path(value).read_text())
            running = {'context': {'position': 0, 'binding': {'vm': 7}}}
            with patch.object(drain, '_private_directory', return_value=directory), \
                    patch.object(drain.Path, 'lstat', return_value=metadata), \
                    patch.object(runtime, 'original_json', side_effect=read_marker):
                yield native, running, marker, record

    def test_actual_transfer_limit_includes_the_last_route_cell_and_preserves_inputs(self):
        with self.route_cell() as (native, running, marker, record):
            before = copy.deepcopy(native)
            self.assertEqual(native['data_transfer']['max_transfers_per_vm'], 5)
            self.assertNotIn('max_data_routes_per_slot', native['bounded_host'])
            self.assertEqual(drain._live_routes(native, running), {
                drain._route_key(record['process']): (marker, record)})
            self.assertEqual(native, before)

    def test_persisted_capacity_and_original_route_identity_cannot_be_substituted(self):
        for field in ('capacity', 'position', 'vm', 'uplink', 'inode', 'path'):
            with self.subTest(field=field), self.route_cell() as (native, running, marker, record):
                context = record['context']
                if field == 'capacity': context['helper_gate']['max_data_routes_per_slot'] = 4
                elif field == 'position': context['position'] = 1
                elif field == 'vm': context['vm'] = 8
                elif field == 'uplink': context['uplink_index'] += 1
                elif field == 'inode': context['helper_gate']['inode'] += 1
                else: context['helper_gate']['path'] += '.foreign'
                marker.write_text(json.dumps(record))
                with self.assertRaises(ValueError):
                    drain._live_routes(native, running)

    def test_control_or_unobserved_process_is_not_an_inflight_data_route(self):
        with self.route_cell() as (native, running, marker, record):
            record['control'] = True
            marker.write_text(json.dumps(record))
            self.assertEqual(drain._live_routes(native, running), {})
            record['control'] = False
            marker.write_text(json.dumps(record))
            with patch.object(drain, '_alive', return_value=False):
                self.assertEqual(drain._live_routes(native, running), {})

    def test_missing_untyped_or_inconsistent_transfer_limits_fail_before_marker_reads(self):
        for fault in ('missing', 'incomplete', 'boolean', 'reserve', 'global'):
            with self.subTest(fault=fault), self.route_cell() as (native, running, _marker, _record):
                if fault == 'missing': del native['data_transfer']
                elif fault == 'incomplete': del native['data_transfer']['max_transfers_per_vm']
                elif fault == 'boolean': native['data_transfer']['max_transfers_per_vm'] = True
                elif fault == 'reserve': native['data_transfer']['control_reserve'] = 5
                else: native['data_transfer']['max_transfers'] = 4
                with patch.object(runtime, 'original_json', side_effect=AssertionError('invalid input reached a route cell')), \
                        self.assertRaises(ValueError):
                    drain._live_routes(native, running)


@unittest.skipUnless(sys.platform == 'linux', 'real Linux pidfd/procfs component')
class DataDrainRelayPauseTests(unittest.TestCase):
    """Real owned Linux child/pipe/pidfd; no VM, Host or exit certificate stub."""
    @contextmanager
    def owned_relay(self):
        child = subprocess.Popen(['/bin/cat'], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        probe = object.__new__(drain.InflightDataDrain)
        probe.pidfd = None
        probe.peer = probe.connection = probe.accepted = None
        probe.deadline = time.monotonic() + 5
        probe._closed = False
        probe._relay_control = dict(pause_requested=False, pause_signal_sent=False,
            pause_confirmed=False, continue_sent=False, gate_release_sent=False,
            cleanup_unknown=False)
        probe.report = {}
        try:
            # Echo is the scheduling barrier: the real pipe child has started
            # and forwarded these bytes before its tracked pidfd is selected.
            child.stdin.write(b'head')
            child.stdin.flush()
            self.assertTrue(drain.select.select([child.stdout], [], [], 5)[0])
            self.assertEqual(child.stdout.read(4), b'head')
            fields = Path(f'/proc/{child.pid}/stat').read_text().rsplit(')', 1)[1].split()
            probe.source = dict(pid=child.pid, start_time_ticks=int(fields[19]),
                boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            probe._bound_source = probe.source.copy()
            probe.pidfd = runtime.open_original_candidate_pidfd(probe.source)
            yield child, probe
        finally:
            probe.close()
            child.stdin.close()
            child.stdout.close()
            # Only this test's unreaped Child, never a looked-up numeric PID.
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)

    def test_begin_stops_the_original_relay_before_publishing_its_arm(self):
        with self.owned_relay() as (child, probe), tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary).resolve()
            directory.chmod(0o700)
            metadata = directory.stat()
            probe.directory = directory
            probe._directory_identity = metadata.st_dev, metadata.st_ino
            probe._directory_owner = metadata.st_uid
            probe._control_files = {}
            probe.native = probe.running = {}
            probe.run_identity = 'a' * 64
            probe.path = '/tmp/controlled-upload'
            route = dict(process=probe.source.copy())
            marker = directory / 'original-route.json'
            marker.write_text(json.dumps(route))
            observer, idle_peer = socket.socketpair()
            connection = Mock(sock=observer)
            def send_prefix(value):
                child.stdin.write(value)
                child.stdin.flush()
            connection.send.side_effect = send_prefix
            def read_prefix(*_args, **_kwargs):
                self.assertTrue(drain.select.select([child.stdout], [], [], 5)[0])
                return child.stdout.read(len(drain.PREFIX))
            probe.client = SimpleNamespace(base_url='http://127.0.0.1:9999',
                data_token='controlled-unused-header',
                sdk=lambda: SimpleNamespace(files=SimpleNamespace(read=read_prefix)))
            try:
                os.close(probe.pidfd)
                probe.pidfd = None
                with patch.object(drain.http.client, 'HTTPConnection', return_value=connection), \
                        patch.object(drain, '_private_directory', Path), \
                        patch.object(drain, '_live_routes', side_effect=[{}, {
                            drain._route_key(route['process']): (marker, route)}]):
                    result = probe.begin()
                fields = Path(f'/proc/{child.pid}/stat').read_text().rsplit(')', 1)[1].split()
                self.assertEqual(fields[0], 'T',
                    'same original relay must be stopped before publishing its arm')
                self.assertFalse(runtime.pidfd_exited(probe.pidfd))
                self.assertTrue(result['guest_prefix_observed'])
                self.assertEqual((directory / 'selected.arm').read_bytes(), probe.arm_bytes)
                self.assertIn('selected.arm', probe._control_files)
            finally:
                probe.close()
                observer.close()
                idle_peer.close()

    def test_pipe_eof_before_stop_barrier_can_legally_exit_the_same_child(self):
        with self.owned_relay() as (child, probe):
            observer, barrier = socket.socketpair()
            arrived = threading.Event()
            errors = queue.Queue()
            def stop_barrier():
                try:
                    arrived.set()
                    self.assertEqual(barrier.recv(1), b'g')
                except BaseException as error:
                    errors.put(error)
            worker = threading.Thread(target=stop_barrier)
            worker.start()
            try:
                self.assertTrue(arrived.wait(5))
                # Hyper cancellation closes pipes before calling the stop
                # barrier. No signal, timeout injection or release is needed
                # for this real pipe consumer's legitimate EOF exit.
                child.stdin.close()
                self.assertTrue(runtime.pidfd_exited(probe.pidfd, 5000))
                self.assertEqual(child.wait(timeout=5), 0)
                self.assertTrue(worker.is_alive())
            finally:
                observer.sendall(b'g')
                worker.join(timeout=5)
                observer.close()
                barrier.close()
            self.assertFalse(worker.is_alive())
            if not errors.empty():
                raise errors.get()

    def test_exact_pause_holds_pipe_eof_until_continue_and_original_wait(self):
        with self.owned_relay() as (child, probe):
            probe._pause_original_relay()
            child.stdin.close()
            probe._require_paused_relay()
            self.assertEqual(probe._relay_state(), 'T')
            self.assertFalse(runtime.pidfd_exited(probe.pidfd))
            waited = queue.Queue()
            entered = threading.Event()
            def original_wait():
                entered.set()
                waited.put(child.wait(timeout=5))
            worker = threading.Thread(target=original_wait)
            worker.start()
            observer, barrier = socket.socketpair()
            try:
                self.assertTrue(entered.wait(5))
                probe._require_paused_relay()
                self.assertTrue(waited.empty())
                probe.peer = observer
                probe.connection = SimpleNamespace(close=lambda: None)
                probe.accepted = {'accepted_version': 2}
                probe.release()
                self.assertEqual(barrier.recv(1), b'g')
                self.assertTrue(probe._relay_control['continue_sent'])
                self.assertTrue(probe._relay_control['gate_release_sent'])
                self.assertTrue(runtime.pidfd_exited(probe.pidfd))
                self.assertEqual(waited.get(timeout=5), 0)
            finally:
                barrier.close()
                worker.join(timeout=5)
            self.assertFalse(worker.is_alive())
            with self.assertRaises(RuntimeError):
                probe.release()

    def test_foreign_boot_start_or_replaced_binding_never_authorizes_a_signal(self):
        for field in ('boot_id', 'start_time_ticks', 'whole_identity'):
            with self.subTest(field=field), self.owned_relay() as (_child, probe):
                if field == 'boot_id': probe.source['boot_id'] = 'foreign'
                elif field == 'start_time_ticks': probe.source['start_time_ticks'] += 1
                else:
                    fields = Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()
                    probe.source = dict(pid=os.getpid(), start_time_ticks=int(fields[19]),
                        boot_id=probe.source['boot_id'])
                with patch.object(drain.signal, 'pidfd_send_signal') as send, \
                        self.assertRaises(ValueError):
                    probe._pause_original_relay()
                send.assert_not_called()
                self.assertFalse(probe._relay_control['pause_requested'])

    def test_exited_relay_and_expired_release_are_refused_without_signalling(self):
        with self.owned_relay() as (child, probe):
            child.stdin.close()
            self.assertTrue(runtime.pidfd_exited(probe.pidfd, 5000))
            self.assertEqual(child.wait(timeout=5), 0)
            with patch.object(drain.signal, 'pidfd_send_signal') as send, \
                    self.assertRaises(ValueError):
                probe._pause_original_relay()
            send.assert_not_called()
        with self.owned_relay() as (_child, probe):
            probe._pause_original_relay()
            observer, barrier = socket.socketpair()
            probe.peer, probe.accepted = observer, {'accepted_version': 2}
            probe.deadline = time.monotonic() - 1
            try:
                with patch.object(drain.signal, 'pidfd_send_signal') as send, \
                        self.assertRaises(TimeoutError):
                    probe.release()
                send.assert_not_called()
                probe._require_paused_relay()
            finally:
                # Failure cleanup may withdraw its own exact test pause even
                # after the scenario deadline. It never asserts exit/success.
                result = probe.close()
                barrier.close()
            self.assertTrue(result['continue_sent'])
            self.assertFalse(result['gate_release_sent'] or result['cleanup_unknown'])

    def test_external_continuation_or_duplicate_pause_cannot_fake_the_held_cut(self):
        with self.owned_relay() as (_child, probe):
            probe._pause_original_relay()
            with patch.object(drain.signal, 'pidfd_send_signal') as send, \
                    self.assertRaises(ValueError):
                probe._pause_original_relay()
            send.assert_not_called()
            signal.pidfd_send_signal(probe.pidfd, signal.SIGCONT)
            while probe._relay_state() == 'T':
                drain._remaining(probe.deadline)
            with self.assertRaises(AssertionError):
                probe._require_paused_relay()

    def test_failure_close_withdraws_only_its_pause_and_is_idempotent(self):
        with self.owned_relay() as (_child, probe):
            probe._pause_original_relay()
            result = probe.close()
            self.assertEqual(result['cleanup'], 'same-pidfd-test-pause-withdrawn')
            self.assertTrue(result['continue_sent'])
            self.assertFalse(result['gate_release_sent'] or result['cleanup_unknown'])
            self.assertIsNone(probe.pidfd)
            with patch.object(drain.signal, 'pidfd_send_signal') as send:
                self.assertEqual(probe.close(), result)
            send.assert_not_called()
            with self.assertRaises(AssertionError):
                probe._continue_original_relay()

    def test_failure_after_stop_signal_does_not_leave_an_unconfirmed_pause(self):
        with self.owned_relay() as (_child, probe):
            send_signal = signal.pidfd_send_signal
            def interrupted_after_stop(descriptor, value):
                send_signal(descriptor, value)
                raise InterruptedError('observer interrupted after its exact signal')
            with patch.object(drain.signal, 'pidfd_send_signal',
                    side_effect=interrupted_after_stop), self.assertRaises(InterruptedError):
                probe._pause_original_relay()
            self.assertTrue(probe._relay_control['pause_requested'])
            self.assertFalse(probe._relay_control['pause_signal_sent'])
            result = probe.close()
            self.assertTrue(result['continue_sent'])
            self.assertFalse(result['pause_confirmed'] or result['gate_release_sent']
                             or result['cleanup_unknown'])

    def test_failure_close_preserves_unknown_identity_without_signalling(self):
        with self.owned_relay() as (_child, probe):
            probe._pause_original_relay()
            original = probe._bound_source.copy()
            probe.source['start_time_ticks'] += 1
            with patch.object(drain.signal, 'pidfd_send_signal') as send:
                result = probe.close()
            send.assert_not_called()
            self.assertTrue(result['cleanup_unknown'])
            self.assertEqual(result['original_process'], original)
            self.assertFalse(result['continue_sent'] or result['gate_release_sent'])
            # The owned-child fixture's finalizer kills/reaps exactly its own
            # still-stopped child; the observer cannot guess continuation.


if __name__ == '__main__':
    unittest.main()
