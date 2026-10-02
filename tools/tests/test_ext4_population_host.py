"""Supervisor contracts only: no HTTP server, VM, mount or privileged cleanup."""
import copy
from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/ext4_population_host.py'
SPEC = importlib.util.spec_from_file_location('ext4_population_host', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def guest_result():
    return dict(passed=True, entries=7, snapshot_sha256='a' * 64,
                cases=[dict(case=name, passed=True, mount_and_loop_gone=True)
                       for name in ('success', 'slow-unmount', 'cp', 'mount')])


def settings():
    return dict(namespace='a' * 16, credentials=[dict(tenant=1, secret='private-key')])


class Fixture:
    def __init__(self, *, unknown_create=False, guest=None, close_fails=False,
                 logical_never_finishes=False, audit_fails=False, capture_fails=False):
        self.calls, self.evidence_rows, self.commands, self.uploads = [], [], [], []
        self.window = 'bw-' + 'a' * 16 + '-1-0'
        self.ident = 'sb-' + 'a' * 16 + '-1'
        self.vm = dict(sandboxID=self.ident, state='Running', cleanupComplete=False,
                       session=dict(sandboxID=self.ident, envdAccessToken='private-session', envdVersion='0.9.0'))
        self.unknown_create, self.close_fails = unknown_create, close_fails
        self.logical_never_finishes, self.audit_fails = logical_never_finishes, audit_fails
        self.capture_fails = capture_fails
        self.deleted, self.now = False, 0
        self.guest = guest_result() if guest is None else guest
        self.sandbox = SimpleNamespace(commands=SimpleNamespace(run=self.command),
                                       files=SimpleNamespace(write=self.upload))

    def request(self, method, path, body=None, **kwargs):
        self.calls.append((method, path, body, kwargs))
        if path == '/sandproof/internal-build-windows':
            return dict(requestWindow=self.window, expiresAtMs=1000)
        if path.endswith('/close'):
            if self.close_fails:
                raise probe.RequestFailed(500)
            return dict(requestWindow=self.window, closed=True)
        if path.endswith('/settle'):
            self.assert_settlement_saved = self.evidence_rows[-1][0] == 'settlement'
            return dict(requestWindow=self.window, settled=True)
        if method == 'POST':
            if self.unknown_create:
                raise TimeoutError('private-session must not leak')
            return self.vm
        if method == 'DELETE':
            self.deleted = True
            return None
        if method == 'GET':
            return dict(self.vm, cleanupComplete=self.deleted and not self.logical_never_finishes)
        raise AssertionError('unexpected test request')

    def evidence(self, name, value):
        self.evidence_rows.append((name, copy.deepcopy(value)))

    def capture(self, ident):
        if self.capture_fails:
            raise ValueError('unavailable')
        return dict(sandbox_id=ident, process=dict(observed_alive=True), network=[1, 2], cgroup=[3, 4])

    def audit(self, identity):
        if self.audit_fails:
            raise ValueError('resource remains')
        return dict(passed=True)

    def command(self, text, **kwargs):
        self.commands.append((text, kwargs))
        return SimpleNamespace(exit_code=0, stdout=json.dumps(self.guest))

    def upload(self, name, incoming, **kwargs):
        self.uploads.append((name, len(incoming.read()), kwargs))

    def sleep(self, seconds):
        self.now += 10  # Deterministic advancement, no real sleep.

    def run(self):
        return probe.supervise(settings(), self.request, lambda session: self.sandbox,
            self.evidence, self.capture, self.audit, clock=lambda: self.now,
            sleep=self.sleep, build_id='b' * 32)


class SupervisorTests(unittest.TestCase):
    def test_original_window_is_durable_before_one_create_and_settlement_before_unpin(self):
        fixture = Fixture()
        report = fixture.run()
        self.assertTrue(report['passed'])
        self.assertEqual([row[0] for row in fixture.evidence_rows],
                         ['intent', 'window', 'create', 'vm', 'identity', 'settlement'])
        create = [row for row in fixture.calls if row[0] == 'POST' and
                  row[1].endswith('b' * 32)]
        self.assertEqual(len(create), 1)
        self.assertEqual(create[0][2], dict(requestWindow=fixture.window, timeout=600, allowInternetAccess=False))
        self.assertTrue(fixture.assert_settlement_saved)
        self.assertEqual(fixture.commands[-1], ('python3 /input/ext4_population_guest.py',
                                               dict(user='root', timeout=300)))
        self.assertEqual({row[0] for row in fixture.uploads},
                         {'/input/' + name for name in probe.GUEST_FILES})
        self.assertNotIn('private-key', json.dumps((report, fixture.evidence_rows)))
        self.assertNotIn('private-session', json.dumps((report, fixture.evidence_rows)))

    def test_unknown_create_never_recreates_and_still_cleans_the_original_id(self):
        fixture = Fixture(unknown_create=True)
        report = fixture.run()
        self.assertFalse(report['passed'])
        self.assertEqual(report['error_type'], 'TimeoutError')
        self.assertTrue(report['cleanup']['passed'])
        self.assertEqual(len([row for row in fixture.calls if row[0] == 'POST' and row[1].endswith('b' * 32)]), 1)
        deletes = [row[1] for row in fixture.calls if row[0] == 'DELETE']
        self.assertEqual(deletes, ['/sandboxes/' + fixture.ident])

    def test_recovered_capture_clears_error_only_after_durable_complete_identity(self):
        fixture = Fixture(unknown_create=True)
        original = fixture.capture
        observed = []

        def capture(ident):
            observed.append(ident)
            if len(observed) == 1:
                raise ValueError('transient observation failure')
            self.assertFalse(fixture.deleted)
            return original(ident)

        fixture.capture = capture
        report = fixture.run()
        self.assertFalse(report['passed'])  # The unknown original create remains unknown.
        self.assertEqual(report['error_type'], 'TimeoutError')
        self.assertTrue(report['cleanup']['passed'])
        self.assertEqual(observed, [fixture.ident, fixture.ident])
        self.assertEqual([name for name, _value in fixture.evidence_rows].count('identity'), 1)
        self.assertEqual(len([row for row in fixture.calls if row[0] == 'POST' and row[1].endswith('b' * 32)]), 1)

    def test_partial_identity_is_recaptured_before_delete_or_kept_cleanup_unknown(self):
        for recovers in (True, False):
            with self.subTest(recovers=recovers):
                fixture = Fixture(unknown_create=True)
                original, observed, audited = fixture.capture, [], []

                def capture(ident):
                    observed.append(ident)
                    self.assertFalse(fixture.deleted)
                    if recovers and len(observed) > 1:
                        return original(ident)
                    return dict(sandbox_id=ident, process=None, network=None)

                fixture.capture = capture
                fixture.audit = lambda identity: audited.append(identity) or dict(passed=True)
                report = fixture.run()
                self.assertFalse(report['passed'])
                self.assertTrue(fixture.deleted)
                self.assertTrue(all(ident == fixture.ident for ident in observed))
                self.assertEqual(len([row for row in fixture.calls if row[0] == 'POST' and row[1].endswith('b' * 32)]), 1)
                names = [name for name, _value in fixture.evidence_rows]
                self.assertIn('identity-partial-1', names)
                if recovers:
                    self.assertTrue(report['cleanup']['passed'])
                    self.assertEqual(names.count('identity'), 1)
                    self.assertTrue(probe.complete_identity(audited[0]))
                else:
                    self.assertFalse(report['cleanup']['passed'])
                    self.assertEqual(report['cleanup']['status'], 'cleanup_unknown')
                    self.assertNotIn('identity', names)
                    self.assertEqual(audited, [])
                    self.assertFalse(any(row[1].endswith('/settle') for row in fixture.calls))

    def test_logical_cleanup_with_unknown_identity_never_unpins_original_anchor(self):
        for partial in (True, False):
            with self.subTest(partial=partial):
                fixture = Fixture(unknown_create=True)
                original = fixture.request

                def request(method, path, body=None, **kwargs):
                    value = original(method, path, body, **kwargs)
                    return dict(value, cleanupComplete=True) if method == 'GET' else value

                def capture(ident):
                    self.assertEqual(ident, fixture.ident)
                    if partial:
                        return dict(sandbox_id=ident, process=None, network=None)
                    raise ValueError('original identity unavailable after cleanup')

                fixture.request, fixture.capture = request, capture
                report = fixture.run()
                self.assertFalse(report['passed'])
                self.assertEqual(report['cleanup']['status'], 'cleanup_unknown')
                self.assertTrue(fixture.deleted)
                self.assertFalse(any(name == 'settlement' for name, _value in fixture.evidence_rows))
                self.assertFalse(any(row[1].endswith('/settle') for row in fixture.calls))
                self.assertEqual(len([row for row in fixture.calls if row[0] == 'POST' and row[1].endswith('b' * 32)]), 1)

    def test_kill_reply_unknown_is_resolved_only_by_original_get_and_audit(self):
        fixture = Fixture()
        original = fixture.request
        def request(method, path, body=None, **kwargs):
            result = original(method, path, body, **kwargs)
            if method == 'DELETE':
                raise TimeoutError('unknown kill reply')
            return result
        fixture.request = request
        report = fixture.run()
        self.assertTrue(report['passed'])
        self.assertTrue(report['cleanup']['passed'])
        self.assertEqual(len([row for row in fixture.calls if row[0] == 'DELETE']), 1)

    def test_failed_guest_result_is_not_changed_to_success_by_cleanup(self):
        for mutate in ('passed', 'duplicate', 'missing', 'mount'):
            value = guest_result()
            if mutate == 'passed':
                value['passed'] = False
            elif mutate == 'duplicate':
                value['cases'][0] = value['cases'][1]
            elif mutate == 'missing':
                value['cases'].pop()
            else:
                value['cases'][0]['mount_and_loop_gone'] = False
            with self.subTest(mutate=mutate):
                fixture = Fixture(guest=value)
                report = fixture.run()
                self.assertFalse(report['passed'])
                self.assertTrue(report['cleanup']['passed'])
                self.assertTrue(fixture.deleted)

    def test_close_unknown_still_attempts_known_kill_but_cannot_settle(self):
        fixture = Fixture(close_fails=True)
        report = fixture.run()
        self.assertFalse(report['passed'])
        self.assertFalse(report['cleanup']['passed'])
        self.assertTrue(fixture.deleted)
        self.assertFalse(any(row[1].endswith('/settle') for row in fixture.calls))

    def test_cleanup_unknown_and_identity_unknown_fail_closed_without_avoiding_kill(self):
        for kwargs in ({'logical_never_finishes': True}, {'audit_fails': True}, {'capture_fails': True}):
            with self.subTest(kwargs=kwargs):
                fixture = Fixture(**kwargs)
                report = fixture.run()
                self.assertFalse(report['passed'])
                self.assertFalse(report['cleanup']['passed'])
                self.assertTrue(fixture.deleted)

    def test_evidence_failure_before_window_dispatch_has_no_external_action(self):
        fixture = Fixture()
        with self.assertRaises(FileExistsError):
            probe.supervise(settings(), fixture.request, lambda session: fixture.sandbox,
                lambda name, value: (_ for _ in ()).throw(FileExistsError()), fixture.capture, fixture.audit)
        self.assertEqual(fixture.calls, [])

    def test_no_anchor_requires_closed_window_before_404_settlement(self):
        fixture = Fixture(unknown_create=True)
        original = fixture.request
        def request(method, path, body=None, **kwargs):
            if method == 'GET':
                fixture.calls.append((method, path, body, kwargs))
                raise probe.RequestFailed(404)
            return original(method, path, body, **kwargs)
        fixture.request = request
        report = fixture.run()
        self.assertFalse(report['passed'])
        self.assertTrue(report['cleanup']['passed'])
        settlement = fixture.evidence_rows[-1][1]
        self.assertEqual(settlement, dict(requestWindow=fixture.window, sandboxID=None))
        methods = [(row[0], row[1].rsplit('/', 1)[-1]) for row in fixture.calls]
        self.assertLess(next(i for i, row in enumerate(methods) if row == ('POST', 'close')),
                        next(i for i, row in enumerate(methods) if row[0] == 'GET'))

    def test_wrong_vm_identity_cannot_trigger_other_sandbox_kill(self):
        fixture = Fixture()
        fixture.vm['sandboxID'] = 'sb-' + 'c' * 16 + '-1'
        report = fixture.run()
        self.assertFalse(report['passed'])
        self.assertFalse(fixture.deleted)


class FileAndConfigurationTests(unittest.TestCase):
    def test_new_evidence_is_private_and_never_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(probe.host, 'trusted_path', side_effect=lambda path: path):
            directory = Path(temporary)
            path = directory / 'intent.json'
            probe.write_new(path, {'version': 1})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                probe.write_new(path, {'version': 2})
            self.assertEqual(json.loads(path.read_text()), {'version': 1})

    def test_invalid_id_and_nonexact_case_set_are_rejected_before_use(self):
        for value in ('B' * 32, 'x', '../escape'):
            fixture = Fixture()
            with self.assertRaises(ValueError):
                probe.supervise(settings(), fixture.request, lambda session: fixture.sandbox,
                    fixture.evidence, fixture.capture, fixture.audit, build_id=value)
            self.assertEqual(fixture.calls, [])
        value = guest_result()
        value['cases'].append(value['cases'][0])
        with self.assertRaises(ValueError):
            probe.validate_guest(value)

    def test_remote_or_other_loopback_url_is_rejected(self):
        for url in ('https://example.org', 'http://127.0.0.1:4001'):
            with self.assertRaises(ValueError):
                probe.validate_configuration({'bind': '127.0.0.1:4000'}, url)

    def test_namespace_scan_includes_fd_only_holder_and_compares_device_and_inode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'original'
            original.write_bytes(b'namespace')
            process = root / 'proc/123'
            (process / 'fd').mkdir(parents=True)
            (process / 'ns').mkdir()
            (process / 'ns/net').write_bytes(b'different namespace')
            (process / 'fd/0').symlink_to(original)
            actual = original.stat()
            path = Path
            with patch.object(probe, 'Path', side_effect=lambda value: root / 'proc' if value == '/proc' else path(value)):
                self.assertEqual(probe.namespace_holders([actual.st_dev, actual.st_ino]), [123])
                self.assertEqual(probe.namespace_holders([actual.st_dev + 1, actual.st_ino]), [])

    def test_fixed_base_bytes_are_bound_to_the_approved_descriptor(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(probe.host, 'trusted_path', side_effect=lambda path: path):
            root = Path(temporary)
            descriptors, value = {}, {}
            for name in ('kernel', 'rootfs'):
                path = root / name
                path.write_bytes(name.encode())
                value[name] = str(path)
                descriptors[name] = dict(path=str(path), size_bytes=path.stat().st_size, sha256=probe.digest(path))
            value['templates'] = dict(aliases={'base': 'builder'}, templates=[dict(
                template_id='builder', owner=None, **descriptors)])
            self.assertEqual(probe.fixed_artifacts(value, 1),
                             {name: descriptors[name]['sha256'] for name in descriptors})
            (root / 'rootfs').write_bytes(b'rootfx')  # Same length, different bytes.
            with self.assertRaisesRegex(ValueError, 'fingerprint'):
                probe.fixed_artifacts(value, 1)

    def test_wal_aware_cleanup_requires_terminal_state_and_no_pending_original_work(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'controller.db'
            identity = dict(sandbox_id='sb-' + 'a' * 16 + '-1', context={'position': 0})
            value = dict(database=str(path))
            with closing(sqlite3.connect(path)) as writer:
                writer.execute('PRAGMA journal_mode=WAL')
                writer.executescript('CREATE TABLE sandboxes(sandbox_id TEXT,state_blob TEXT);'
                    'CREATE TABLE outbox(sandbox_id TEXT,dispatch_state TEXT);'
                    'CREATE TABLE private_build_anchors(sandbox_id TEXT);'
                    'CREATE TABLE controller_host_slots(position INTEGER,context_json TEXT);')
                writer.execute('INSERT INTO controller_host_slots VALUES(0,?)', (json.dumps(identity['context']),))
                state = dict(phase='Destroyed', terminal=True, current_vm=None, net_slot=None)
                writer.execute('INSERT INTO sandboxes VALUES(?,?)', (identity['sandbox_id'], json.dumps(state)))
                writer.execute("INSERT INTO outbox VALUES(?,'dispatched')", (identity['sandbox_id'],))
                writer.commit()
                with self.assertRaisesRegex(ValueError, 'unresolved'):
                    probe.controller_cleanup(value, identity)
                writer.execute("UPDATE outbox SET dispatch_state='receipted'")
                writer.commit()
                self.assertEqual(probe.controller_cleanup(value, identity)['pending_actions'], 0)
                writer.execute('INSERT INTO private_build_anchors VALUES(?)', (identity['sandbox_id'],))
                writer.commit()
                with self.assertRaisesRegex(ValueError, 'unresolved'):
                    probe.controller_cleanup(value, identity)


if __name__ == '__main__':
    unittest.main()
