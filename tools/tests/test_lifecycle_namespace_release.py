"""Original SQL/log/namespace consumers with controlled product facts; no VM."""
import copy
from contextlib import ExitStack, contextmanager
import errno
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace as NS
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tests/system'))
import lifecycle_namespace_release as held
import lifecycle_unload_capture_faults as capture
import lifecycle_unload_runtime as runtime
import lifecycle_unload_scenario as scene
import lifecycle_unload_stop_pending as pending
from tools.tests import test_lifecycle_unload_stop_pending as original


def fixture(root):
    f = original.fixture(root)
    p = held.NamespaceReleaseProbe.__new__(held.NamespaceReleaseProbe)
    p.__dict__.update(f.probe.__dict__)
    p.record = copy.deepcopy(f.rows[1])
    p.namespace_reference = Mock(fd=100000)
    p.namespace_reference.explicitly_closed = False
    p.namespace_reference.verify.return_value = dict(name=p.prior['source']['resource_id'], device=2, inode=3)
    p.namespace_checks = []
    p.namespace_release = None
    p.observe_peer = Mock(return_value={'original': 'peer-progress'})
    p.runtime.services.processes.logs = root
    (root / 'host-0.log').write_bytes(b'')
    os.chmod(root / 'host-0.log', 0o600)
    f.probe = p
    f.node_record = dict(position=1, helper_gate={'controlled': 'original'},
                         value=dict(context=copy.deepcopy(p.prior['context']), phase='Occupied', retirement=None))
    return f


def witness(f, *, current=False):
    context = copy.deepcopy(f.probe.prior['context'])
    if not current:
        context['binding'].update(generation=1, vm=1, network=1)
        context['position'] = 0
    return dict(context=context, input_sequence=7)


def database(f, saved=None):
    p = f.probe
    raw = json.dumps(dict(sessions=[dict(id=p.client.session, current_version=4, terminal=False,
        local=p.prior['local'], entries=[p.unload_entry])])).encode()
    with sqlite3.connect(p.runtime.services.native_pools[0]['database']) as conn:
        conn.executescript('CREATE TABLE controller_lifecycle_inbox(id INTEGER,state_json BLOB,sha256 BLOB,reserve_bytes INTEGER);'
            'CREATE TABLE outbox(id INTEGER,sandbox_id TEXT,operation_id TEXT,action_json BLOB,dispatch_state TEXT,receipt_json BLOB);'
            'CREATE TABLE controller_release_witnesses(sandbox_id TEXT PRIMARY KEY,witness_json BLOB);')
        conn.execute('INSERT INTO controller_lifecycle_inbox VALUES(1,?,?,65536)', (raw, hashlib.sha256(raw).digest()))
        identity = f.stop['identity']
        conn.execute('INSERT INTO outbox VALUES(1,?,?,?,\'receipted\',NULL)',
                     (identity['sandbox'], identity['operation'], json.dumps(f.stop).encode()))
        if saved is not None:
            conn.execute('INSERT INTO controller_release_witnesses VALUES(?,?)', (p.config['sandbox'], json.dumps(saved).encode()))


@contextmanager
def read_only(f):
    path = Path(f.probe.runtime.services.native_pools[0]['database'])
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as conn:
        conn.execute('BEGIN')
        yield conn


def fault(f, kind, *, generation=None):
    binding = f.probe.prior['context']['binding']
    return ('controller_fault scope=Instance stage=ReleaseSlot kind=' + kind +
        ' code=None io_kind=None sandbox=Some(' + str(binding['sandbox']) + ') operation=None generation=Some(' +
        str(binding['generation'] if generation is None else generation) + ') attempt=None vm=Some(' +
        str(binding['vm']) + ') epoch=None command=None\n').encode()


class ReadOnlyReleaseTests(unittest.TestCase):
    def test_original_sql_snapshot_allows_only_absent_or_older_release_witness(self):
        for saved in ('absent', 'older', 'current', 'owner', 'generation', 'typed', 'large'):
            with self.subTest(saved=saved), tempfile.TemporaryDirectory() as directory:
                f = fixture(Path(directory)); p = f.probe
                value = None if saved == 'absent' else witness(f, current=saved == 'current')
                if saved == 'owner': value['context']['binding']['owner'] += 1
                if saved == 'generation': value['context']['binding']['generation'] += 1
                if saved == 'typed': value['context']['binding']['owner'] = True
                if saved == 'large': value['padding'] = 'x' * 65536
                database(f, value)
                with read_only(f) as conn:
                    with self.assertRaises(sqlite3.OperationalError): conn.execute('DELETE FROM controller_release_witnesses')
                    capacity = p.runtime.services.host_configs[0]['native']['maintenance']['capacity']
                    if saved in ('absent', 'older'):
                        self.assertEqual(p._witness(conn, p.prior['local'], capacity), value)
                    else:
                        with self.assertRaises(ValueError): p._witness(conn, p.prior['local'], capacity)

    def test_actual_read_only_pending_snapshot_requires_old_full_pin_charge_and_nonfree_position(self):
        for failure in ('none', 'free', 'context', 'typed-position', 'full', 'charge', 'pin', 'intent'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                f = fixture(Path(directory)); p = f.probe
                if failure == 'free': f.node_record['value']['phase'] = 'Free'
                if failure == 'context': f.node_record['value']['context']['binding']['vm'] += 1
                if failure == 'typed-position': f.node_record['position'] = True
                if failure == 'full': f.rows[1]['capture']['completed'][0] += 1
                if failure == 'charge': f.rows[1]['capture']['reserved_material_bytes'] = 0
                if failure == 'pin': f.rows[0]['restore']['material_use'] = False
                database(f, witness(f))
                if failure == 'intent': p.unload_entry['intent']['center']['center'] = 'different'
                stack.enter_context(patch.object(capture, 'private_path', side_effect=lambda path, **_: Path(path)))
                stack.enter_context(patch.object(capture, 'checked_source_records', return_value=f.rows))
                stack.enter_context(patch.object(held, 'original_json', return_value=f.node_record))
                stack.enter_context(patch.object(held, 'pidfd_exited', return_value=True))
                if failure == 'none':
                    result = p._held_state(p.deadline)
                    self.assertFalse(result['public_completion'])
                    self.assertGreater(result['material_charge'], 0)
                    self.assertGreater(result['backing_charge'], 0)
                else:
                    with self.assertRaises(ValueError): p._held_state(p.deadline)

    def test_matching_confirm_release_is_an_immediate_forbidden_state_not_a_timeout(self):
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            f = fixture(Path(directory)); p = f.probe; database(f, witness(f, current=True))
            stack.enter_context(patch.object(capture, 'private_path', side_effect=lambda path, **_: Path(path)))
            stack.enter_context(patch.object(capture, 'checked_source_records', return_value=f.rows))
            stack.enter_context(patch.object(held, 'pidfd_exited', return_value=True))
            sleep = stack.enter_context(patch.object(held.time, 'sleep'))
            p._faults = Mock(return_value=set())
            with self.assertRaisesRegex(ValueError, 'confirmed released'):
                p._wait_attempt({}, p.deadline, completed_parent=True)
            sleep.assert_not_called(); p._faults.assert_not_called(); p.observe_peer.assert_not_called()

    def test_original_pending_full_request_is_saved_before_stop_and_cannot_change(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory)); p = f.probe; p.unload_entry = None
            p._facts(f.snapshot)
            self.assertEqual(p.unload_entry, f.snapshot[0])
            changed = copy.deepcopy(f.snapshot); changed[0]['intent']['center']['center'] = 'another-center'
            with self.assertRaises(ValueError): p._facts(changed)


class HostLogTests(unittest.TestCase):
    def test_complete_exact_context_faults_after_mark_distinguish_node_attempt_from_parent_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory)); p = f.probe; path = Path(directory) / 'host-0.log'
            path.write_bytes(fault(f, 'UnresolvedAction'))
            mark = p._log_mark()
            with path.open('ab') as output:
                output.write(fault(f, 'IdentityMismatch', generation=99))
                output.write(fault(f, 'IdentityMismatch'))
                output.write(fault(f, 'UnresolvedAction')[:-1])
            self.assertEqual(p._faults(mark), {'IdentityMismatch'})
            with path.open('ab') as output: output.write(b'\n')
            self.assertEqual(p._faults(mark), {'IdentityMismatch', 'UnresolvedAction'})
            original_read = os.read
            with patch.object(held.os, 'read', side_effect=lambda fd, size: original_read(fd, min(size, 7))):
                self.assertEqual(p._faults(mark), {'IdentityMismatch', 'UnresolvedAction'})

    def test_replaced_truncated_or_oversize_log_is_not_release_evidence(self):
        for failure in ('replaced', 'truncated', 'large'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                f = fixture(Path(directory)); p = f.probe; path = Path(directory) / 'host-0.log'
                path.write_bytes(b'old\n'); mark = p._log_mark()
                if failure == 'replaced': path.rename(path.with_suffix('.old')); path.write_bytes(fault(f, 'IdentityMismatch'))
                if failure == 'truncated': path.write_bytes(b'')
                if failure == 'large':
                    with path.open('ab') as output: output.truncate(mark['offset'] + 1024**2 + 1)
                with self.assertRaises(ValueError): p._faults(mark)

    def test_parent_rejection_wait_checks_current_sql_even_when_node_already_logged(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory)); p = f.probe
            p._held_state = Mock(return_value={'original': 'pending'})
            p._faults = Mock(side_effect=[{'IdentityMismatch'}, {'IdentityMismatch', 'UnresolvedAction'}])
            with patch.object(held.time, 'sleep') as sleep:
                result = p._wait_attempt({}, p.deadline, completed_parent=True)
            self.assertEqual(p._held_state.call_count, 3)
            sleep.assert_called_once()
            self.assertTrue(result['actual_journal_rejection_completed'])


class ReopenAndCompletionTests(unittest.TestCase):
    def test_actual_audit_callback_requires_original_releasing_and_marks_only_after_old_host_exit(self):
        for failure in ('none', 'free', 'lifetime', 'local', 'witness', 'same-process'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                f = fixture(Path(directory)); p = f.probe
                database(f, witness(f, current=failure == 'witness'))
                old, new = Mock(pid=70), Mock(pid=71)
                old.poll.return_value = None
                p.runtime.services.processes.children = {'host-0': old}
                audit = dict(host_slots=[(p.prior['context'], 'Free' if failure == 'free' else 'Releasing')])
                local, lifetime = copy.deepcopy(p.prior['local']), copy.deepcopy(p.prior['lifetime'])
                if failure == 'local': local['sandbox'] += 1
                if failure == 'lifetime': lifetime['end_at_ms'] += 1
                def snapshot(session, deadline, observe):
                    self.assertEqual(session, p.client.session)
                    old.poll.return_value = 0
                    with read_only(f) as conn:
                        result = observe(audit, local, lifetime, conn)
                    with (Path(directory) / 'host-0.log').open('ab') as output: output.write(fault(f, 'UnresolvedAction'))
                    p.runtime.services.processes.children['host-0'] = old if failure == 'same-process' else new
                    return result
                p.runtime._host_snapshot = snapshot
                p._held_state = Mock(return_value={'original': 'pending'})
                if failure == 'none':
                    result, mark = p._audit_held(p.deadline)
                    self.assertEqual(result['original_sql_phase'], 'Releasing')
                    self.assertEqual(mark['offset'], 0)
                    self.assertEqual(p._faults(mark), {'UnresolvedAction'})
                else:
                    with self.assertRaises(ValueError): p._audit_held(p.deadline)

    def test_both_real_attempt_checks_reopen_and_peer_must_precede_explicit_close(self):
        for failure in ('none', 'first-attempt', 'first-audit', 'parent-attempt', 'second-audit', 'peer', 'last-state'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
                f = fixture(Path(directory)); p = f.probe; events = []
                def event(name, result=None):
                    def run(*args, **kwargs):
                        events.append(name)
                        if name == failure: raise ValueError('unknown ' + name)
                        return result
                    return run
                stack.enter_context(patch.object(pending.PendingStopProbe, 'finish', side_effect=event('restart', {'same': 'config'})))
                waits = iter((event('first-attempt', {}), event('parent-attempt', {})))
                p._wait_attempt = Mock(side_effect=lambda *a, **k: next(waits)(*a, **k))
                audits = iter((event('first-audit', ({}, {})), event('second-audit', ({}, {}))))
                p._audit_held = Mock(side_effect=lambda *a: next(audits)(*a))
                p.observe_peer.side_effect = event('peer', {'original': 'peer'})
                p._held_state = Mock(side_effect=event('last-state', {'held_namespace': {'inode': 3}}))
                p.namespace_reference.close.side_effect = event('close')
                if failure == 'none':
                    result = p.finish(p.deadline)
                    self.assertEqual(events, ['restart', 'first-attempt', 'first-audit', 'parent-attempt', 'second-audit', 'peer', 'last-state', 'close'])
                    self.assertFalse(result['capacity_released']); self.assertFalse(result['public_completion'])
                    p.namespace_reference.close.assert_called_once_with(explicit=True)
                    self.assertFalse(p._wait_attempt.call_args_list[0].kwargs['completed_parent'])
                    self.assertTrue(p._wait_attempt.call_args_list[1].kwargs['completed_parent'])
                else:
                    with self.assertRaisesRegex(ValueError, 'unknown'): p.finish(p.deadline)
                    p.namespace_reference.close.assert_not_called()
                    self.assertIsNone(p.namespace_release)

    def test_failure_close_only_releases_observer_handles_and_never_supplies_confirmation(self):
        with tempfile.TemporaryDirectory() as directory:
            f = fixture(Path(directory)); p = f.probe
            with self.assertRaises(ValueError): p.cleanup()
            with patch.object(pending.PendingStopProbe, 'close', return_value={'physical_exit_inferred': False}) as parent:
                result = p.close()
            parent.assert_called_once(); p.namespace_reference.close.assert_called_once_with()
            self.assertFalse(result['namespace_close_is_release_evidence'])
            self.assertFalse(p.failure_state()['public_completion'])
            self.assertFalse(p.namespace_reference.explicitly_closed)


class OriginalNamespaceDescriptorTests(unittest.TestCase):
    def test_named_handle_is_read_only_noninheritable_and_refuses_replacement_or_unprotected_path(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'namespace'; path.write_bytes(b'controlled named handle')
            os.chmod(path, 0o600)
            meta = path.stat()
            before = NS(st_mode=meta.st_mode, st_uid=0, st_dev=meta.st_dev, st_ino=meta.st_ino)
            with patch.object(Path, 'lstat', side_effect=[before, before]):
                descriptor = held.open_namespace_file(path)
            try:
                self.assertFalse(os.get_inheritable(descriptor))
                with self.assertRaises(OSError): os.write(descriptor, b'no write authority')
            finally: os.close(descriptor)
            changed = NS(**dict(vars(before), st_ino=before.st_ino + 1))
            with patch.object(Path, 'lstat', side_effect=[before, changed]):
                with self.assertRaisesRegex(ValueError, 'changed'): held.open_namespace_file(path)
            for fields in ({'st_uid': 1}, {'st_mode': meta.st_mode | 0o022}):
                unsafe = NS(**dict(vars(before), **fields))
                with patch.object(Path, 'lstat', return_value=unsafe), patch.object(held.os, 'open') as opened:
                    with self.assertRaises(ValueError): held.open_namespace_file(path)
                    opened.assert_not_called()

    @contextmanager
    def child(self):
        child = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stdin.buffer.read()'], stdin=subprocess.PIPE)
        descriptor = os.pidfd_open(child.pid)
        source = dict(pid=child.pid,
            start_time_ticks=int((Path('/proc') / str(child.pid) / 'stat').read_text().rsplit(')', 1)[1].split()[19]),
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
        try:
            yield child, descriptor, source
        finally:
            child.stdin.close()
            child.wait(timeout=5)
            os.close(descriptor)

    def test_real_original_proc_namespace_fd_survives_positive_child_exit_and_is_not_inherited(self):
        with tempfile.TemporaryDirectory() as directory, self.child() as (child, pidfd, source):
            f = fixture(Path(directory)); p = f.probe
            p.prior['source'] = dict(source, resource_id=p.prior['source']['resource_id'])
            path = '/proc/' + str(child.pid) + '/ns/net'; meta = os.stat(path)
            record = dict(resource_id=p.prior['source']['resource_id'], namespace_device=meta.st_dev, namespace_inode=meta.st_ino)
            with patch.object(held, 'original_json', return_value=record), patch.object(held, 'open_namespace_file', side_effect=lambda _: os.open(path, os.O_RDONLY | os.O_CLOEXEC)):
                reference = held.NamespaceReference(p.runtime, p.prior, pidfd)
            descriptor = reference.fd
            self.assertFalse(os.get_inheritable(descriptor))
            child.stdin.close(); child.wait(timeout=5)
            self.assertTrue(runtime.pidfd_exited(pidfd))
            self.assertEqual(reference.verify()['inode'], meta.st_ino)
            reference.close(explicit=True)
            self.assertTrue(reference.explicitly_closed)
            with self.assertRaises(OSError) as error: os.fstat(descriptor)
            self.assertEqual(error.exception.errno, errno.EBADF)

    def test_wrong_record_namespace_or_process_never_keeps_an_acquired_fd(self):
        for failure in ('inode', 'resource', 'typed-device', 'process'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory, self.child() as (child, pidfd, source):
                f = fixture(Path(directory)); p = f.probe
                p.prior['source'] = dict(source, resource_id=p.prior['source']['resource_id'])
                path = '/proc/' + str(child.pid) + '/ns/net'; meta = os.stat(path)
                record = dict(resource_id=p.prior['source']['resource_id'], namespace_device=meta.st_dev, namespace_inode=meta.st_ino)
                if failure == 'inode': record['namespace_inode'] += 1
                if failure == 'resource': record['resource_id'] = 'another'
                if failure == 'typed-device': record['namespace_device'] = float(meta.st_dev)
                if failure == 'process': p.prior['source']['start_time_ticks'] += 1
                acquired = []
                def open_fd(_):
                    fd = os.open(path, os.O_RDONLY | os.O_CLOEXEC); acquired.append(fd); return fd
                with patch.object(held, 'original_json', return_value=record), patch.object(held, 'open_namespace_file', side_effect=open_fd):
                    with self.assertRaises(ValueError): held.NamespaceReference(p.runtime, p.prior, pidfd)
                if failure == 'process': self.assertEqual(acquired, [])
                for fd in acquired:
                    with self.assertRaises(OSError): os.fstat(fd)


class OriginalScenarioTests(unittest.TestCase):
    def exercise(self, failure=None):
        events = []; clients = {}; lifetime = dict(started_at_ms=1, end_at_ms=2, max_end_at_ms=3, startup_deadline_ms=2)
        for owner in ('1', '2'):
            client = Mock(spec=scene.PublicUnloadScenario); client.owner = owner; client.session = 'session-' + owner
            client.create_acceptance = {}; client.current_acceptance = {}; client.unload_acceptance = {'key': {}}
            client.concurrent_unload_replies = [{}, {}]; client.resume_request = {}; client.witness = {}
            client.timings = [{'sent_monotonic': time.monotonic()}]; client.last_http_status = None
            client.failure_step = 'controlled'; client.timing_report.return_value = []
            client.wait_running.return_value = {'phase': 'running'}; client.wait_unloaded.return_value = {'phase': 'unloaded'}
            client.query.return_value = {'phase': 'running'}
            for name in ('create', 'unload', 'resume', 'destroy', 'wait_cleaned', 'run_public_workload', 'assert_resumed_workload', 'assert_running_workload'):
                getattr(client, name).side_effect = lambda *_, n=name, o=owner, **__: events.append(n + '-' + o) or {}
            clients[owner] = client
        calls = 0
        def running(session, *_, **__):
            nonlocal calls
            if session == 'session-1': calls += 1
            return dict(session=session, lifetime=lifetime,
                context={'position': 0 if session == 'session-2' or calls == 1 else 1})
        probe = Mock(spec=held.NamespaceReleaseProbe)
        for name in ('arm', 'after_acceptance', 'cleanup', 'close'):
            getattr(probe, name).side_effect = lambda *_, n=name, **__: events.append(n) or {}
        if failure == 'stop': probe.after_acceptance.side_effect = ValueError('unknown original Stop')
        def finish(deadline):
            events.append('finish')
            if failure == 'release': raise ValueError('unknown actual release')
            constructor.call_args.kwargs['observe_peer']()
            events.append('explicit-namespace-close')
            return {'namespace_descriptor_closed': True, 'public_completion': False}
        probe.finish.side_effect = finish
        probe.failure_state.return_value = {'original_acceptance': 'preserved', 'namespace_held': True}
        if failure == 'peer': clients['2'].assert_running_workload.side_effect = ValueError('original peer stalled')
        with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
            runner = NS(root=Path(directory), owner_key='original', foreign_key='peer',
                services=NS(sdk_url='http://127.0.0.1:1'), measurement_identity=lambda: {}, start=lambda _: None,
                create_parameters=lambda **_: {}, record_running=Mock(side_effect=running),
                unload_evidence=Mock(side_effect=lambda *_: events.append('product-unloaded') or dict(facts=lifetime, original_position={'position': 0}, original_capture_responsibilities=1)),
                restart_managers=Mock(return_value={'original_unload_operation': {}}), confirm_running=Mock(return_value={}),
                destroy_evidence=Mock(return_value={}), settlement_evidence=Mock(return_value={}),
                finish=Mock(side_effect=lambda _: events.append('runtime-finish')), fail_resume_gate=Mock(return_value='unarmed'),
                close_restore_gate=Mock(), close_capture_cancel_gate=Mock())
            stack.enter_context(patch.object(runtime, 'prepare', return_value=runner))
            stack.enter_context(patch.object(scene, 'PublicUnloadScenario', side_effect=lambda *_, owner='1': clients[owner]))
            stack.enter_context(patch.object(scene, 'check_center_configuration', return_value={'deployment': 'original'}))
            constructor = stack.enter_context(patch.object(held, 'NamespaceReleaseProbe', return_value=probe))
            args = NS(scenario='capture-namespace-release', scenario_timeout=60, activation_gate_timeout_ms=60000)
            if failure is not None:
                with self.assertRaises(ValueError): scene.exercise(args)
                report = json.loads((Path(directory) / 'lifecycle-unload-report.json').read_text())
                self.assertFalse(report['complete']); self.assertEqual(len(report['known_sessions']), 2)
                self.assertTrue(report['stop_pending_failure_state']['namespace_held'])
                clients['1'].destroy.assert_not_called(); clients['2'].destroy.assert_not_called()
                probe.cleanup.assert_not_called(); runner.settlement_evidence.assert_not_called()
                self.assertNotIn('explicit-namespace-close', events)
                self.assertEqual(clients['1'].resume.call_count, 1)
            else:
                report = scene.exercise(args)
                self.assertTrue(report['complete']); self.assertEqual(len(report['cycles']), 2)
                self.assertTrue(report['namespace_release']['namespace_descriptor_closed'])
                self.assertIn('namespace descriptor remained held', report['acceptance_cases']['G6'])
                self.assertEqual(clients['1'].resume.call_count, 2)
                self.assertEqual(clients['2'].assert_running_workload.call_count, 2)
                clients['2'].assert_continued_workload.assert_not_called()
                self.assertLess(events.index('run_public_workload-2'), events.index('arm'))
                self.assertLess(events.index('assert_running_workload-2'), events.index('explicit-namespace-close'))
                self.assertLess(events.index('explicit-namespace-close'), events.index('destroy-2'))
                self.assertLess(events.index('destroy-2'), [i for i, event in enumerate(events) if event == 'resume-1'][1])
                self.assertEqual(events[events.index('explicit-namespace-close') + 1], 'product-unloaded')
                probe.cleanup.assert_called_once(); runner.settlement_evidence.assert_called_once()
            self.assertEqual(clients['1'].unload.call_count, 2)
            constructor.assert_called_once(); probe.arm.assert_called_once(); probe.after_acceptance.assert_called_once()
            runner.finish.assert_called_once(); probe.close.assert_called_once()
            self.assertLess(events.index('runtime-finish'), events.index('close'))

    def test_actual_selected_scene_keeps_original_peer_until_product_release_and_completes_both_restores(self):
        self.exercise()

    def test_unknown_stop_release_or_peer_does_not_continue_or_force_destroy_either_owner(self):
        for failure in ('stop', 'release', 'peer'):
            with self.subTest(failure=failure): self.exercise(failure)


if __name__ == '__main__':
    unittest.main()
