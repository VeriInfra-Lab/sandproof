"""Supervisor contracts only: no HTTP server, VM, mount or privileged cleanup."""
import copy
from contextlib import ExitStack
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/ext4_population_host.py'
SPEC = importlib.util.spec_from_file_location('ext4_population_host', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def guest_result():
    return dict(passed=True, entries=7, snapshot_sha256='a' * 64,
                cases=[dict(case=name, passed=True, mount_and_loop_gone=True)
                       for name in ('success', 'slow-unmount', 'cp', 'mount')])


def settings():
    return dict(deployment='test-deployment', owner=1)


class Fixture:
    def __init__(self, *, unknown_create=False, guest=None, close_fails=False,
                 logical_never_finishes=False, audit_fails=False, capture_fails=False):
        self.calls, self.evidence_rows, self.commands, self.uploads = [], [], [], []
        self.window = 'bw-test-deployment-1-0'
        self.ident = 'sandbox-7'
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

    def test_retired_host_configuration_is_rejected_without_upgrade(self):
        for field in ('bind', 'credentials', 'internal_build_vms', 'build_retry_windows'):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'retired'):
                probe.validate_configuration({field: True}, {}, {})

    def test_final_private_boundary_and_unchanged_lifetime_are_explicit(self):
        from tools.tests.test_template_physical_storage import configuration
        from tools.tests.test_template_build_config import HostUidRangeTests
        with tempfile.TemporaryDirectory() as temporary:
            _runtime, builder, _public, _profile = configuration(Path(temporary))
            uid_settings = HostUidRangeTests().settings()
            builder.update(uid_base=uid_settings['uid_base'], data_transfer=uid_settings['data_transfer'],
                max_timeout_secs=600, max_lifetime_secs=600)
            builder['bounded_host'].update(uid_settings['bounded_host'], console_policy='discard')
            center = dict(domain='example.invalid', private_builds={'policy': dict(
                max_timeout_secs=600, max_lifetime_secs=600)},
                private_build_boundary={'bind': '127.0.0.1:4400'})
            build = {'center': {'private_address': '127.0.0.1:4400', 'domain': 'example.invalid'}}
            self.assertEqual(list(probe.validate_configuration(builder, center, build)), [1000, 1001])
            for mutate in ('address', 'domain', 'native_lifetime', 'center_lifetime', 'capability'):
                b, c, config = copy.deepcopy((builder, center, build))
                if mutate == 'address':
                    config['center']['private_address'] = '127.0.0.1:4401'
                elif mutate == 'domain':
                    config['center']['domain'] = 'other.invalid'
                elif mutate == 'native_lifetime':
                    b['max_lifetime_secs'] = 599
                elif mutate == 'center_lifetime':
                    c['private_builds']['policy']['max_timeout_secs'] = 599
                else:
                    c['private_builds'] = None
                with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                    probe.validate_configuration(b, c, config)

    def test_cli_has_one_fresh_fixture_and_no_retired_public_host_entry(self):
        parser = probe.arguments()
        names = {flag for action in parser._actions for flag in action.option_strings}
        self.assertTrue({'--center', '--host', '--bounded-policy', '--run-root',
                         '--service-cgroup-record', '--registry-limits', '--build-limits'} <= names)
        self.assertFalse({'--server', '--config', '--api-url', '--legacy'} & names)

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

    def test_private_client_uses_original_backend_and_canonical_owner_not_user_key(self):
        from unittest.mock import Mock
        backend = Mock()
        backend.request.return_value = {'requestWindow': 'original'}
        client = probe.Client(backend, 3)
        self.assertEqual(client('POST', '/sandproof/internal-build-windows', {}, timeout=10),
                         {'requestWindow': 'original'})
        backend.request.assert_called_once_with('builder', 'POST',
            '/sandproof/internal-build-windows', {}, owner=3, timeout=10)
        backend.request.side_effect = probe.BuildError(404, 'NotFound')
        with self.assertRaises(probe.RequestFailed) as observed:
            client('GET', '/same-original-tuple')
        self.assertEqual(observed.exception.code, 404)
        for owner in ('3', True, 0):
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                probe.Client(backend, owner)

    def test_original_services_own_planned_audits_not_live_sql_or_external_attach(self):
        from unittest.mock import Mock
        services = Mock(host_configs=[{}, {'inbox': {'node': node_identity()}}])
        services.planned_audit.return_value = {'host': 'checked'}
        deployment = Mock(settings=[{}, {'namespace': 'builder'}], services=services)
        deployment.center_snapshot.return_value = {'center': 'checked'}
        observer = probe.Observations(deployment, 1)
        self.assertEqual(observer.snapshots(), ({'center': 'checked'}, {'host': 'checked'}))
        services.planned_audit.assert_called_once_with('host-1')
        deployment.center_snapshot.assert_called_once_with()


def node_identity():
    return dict(deployment_id='test-deployment', node_id='host-builder', node_generation=1)


def audit_fixture():
    create = dict(protocol_version=2, node=node_identity(), tenant_id='1', sandbox_id='sandbox-7',
                  operation_id='operation-9', intent_version=1, retention_slot=1, request_window=1)
    current = dict(create, operation_id='operation-10', intent_version=2)
    response = dict(identity=current, observation_version=5, fact={'kind': 'cleaned'})
    local = dict(sandbox=41, owner=23, operation=89)  # Deliberately not GLOBAL IDs/owner.
    context = dict(position=0, expected_previous=None,
                   binding=dict(sandbox=41, owner=23, vm=77))
    center = dict(schema_version=2, deployment='test-deployment',
        versions=dict(internal_protocol=2, center_store=6, host_store=16),
        retention_lanes=[dict(node=node_identity(), retention_slot=1, retired_through=0, last_retired=None)],
        sandboxes=[dict(create={'identity': create}, private=True, released=True,
            operations=[{'response': response}], terminal_confirmation=response)])
    host = dict(schema_version=2, versions=copy.deepcopy(center['versions']), node=node_identity(),
        lanes=[dict(retention_slot=1, retired_through=0, last_retired=None)],
        operations=[dict(identity=current, current=True, local=local, response=response, confirmed=response)],
        host_slots=[[context, 'Free']],
        native=[dict(sandbox=41, owner=23, phase='Destroyed', terminal=True, current_vm=None,
                     resources=dict(vms=[], slots=[], snapshots=[]))])
    identity = dict(sandbox_id='sandbox-7', create_identity=create, local=local, context=context)
    return center, host, identity


class CheckedMappingAndCleanupTests(unittest.TestCase):
    def test_global_suffix_and_owner_never_stand_in_for_native_binding(self):
        center, host, identity = audit_fixture()
        self.assertEqual(probe.mapped_context(center, host, 'sandbox-7', node=node_identity(), owner=1),
                         (identity['create_identity'], identity['local'], identity['context']))
        self.assertNotEqual(identity['local']['sandbox'], 7)
        self.assertNotEqual(identity['local']['owner'], 1)
        for mutate in ('generation', 'owner', 'private', 'local', 'context'):
            c, h = copy.deepcopy(center), copy.deepcopy(host)
            if mutate == 'generation':
                h['node']['node_generation'] = 2
            elif mutate == 'owner':
                c['sandboxes'][0]['create']['identity']['tenant_id'] = '2'
            elif mutate == 'private':
                c['sandboxes'][0]['private'] = False
            elif mutate == 'local':
                h['operations'][0]['local'] = None
            else:
                h['host_slots'][0][0]['binding']['owner'] = 1
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                probe.mapped_context(c, h, 'sandbox-7', node=node_identity(), owner=1)

    def test_exact_terminal_confirmations_and_original_free_position_are_mandatory(self):
        center, host, identity = audit_fixture()
        self.assertEqual(probe.controller_cleanup(center, host, identity)['terminal_status'], 'cleaned')
        for mutate in ('unconfirmed', 'missing', 'live', 'resource', 'context', 'occupied'):
            c, h = copy.deepcopy(center), copy.deepcopy(host)
            if mutate == 'unconfirmed':
                h['operations'][0]['confirmed'] = None
            elif mutate == 'missing':
                c['sandboxes'] = []
                h['native'] = []
            elif mutate == 'live':
                h['native'][0]['terminal'] = False
            elif mutate == 'resource':
                h['native'][0]['resources']['vms'] = [77]
            elif mutate == 'context':
                h['host_slots'][0][0]['binding']['vm'] = 78
            else:
                h['host_slots'][0][1] = 'Occupied'
            with self.subTest(mutate=mutate), self.assertRaises(ValueError):
                probe.controller_cleanup(c, h, identity)

    def test_exact_both_authority_stamps_allow_retired_records_not_plain_absence(self):
        center, host, identity = audit_fixture()
        stamp = copy.deepcopy(host['operations'][0]['response'])
        center['sandboxes'], host['operations'], host['native'] = [], [], []
        for lanes in (center['retention_lanes'], host['lanes']):
            lanes[0].update(retired_through=1, last_retired=copy.deepcopy(stamp))
        self.assertEqual(probe.controller_cleanup(center, host, identity)['terminal_status'],
                         'retired_terminal_slot')
        host['lanes'][0]['retired_through'] = 0
        host['lanes'][0]['last_retired'] = None
        with self.assertRaises(ValueError):
            probe.controller_cleanup(center, host, identity)


class OwnedFixtureControlFlowTests(unittest.TestCase):
    def test_original_services_stop_but_unknown_leaf_and_evidence_are_retained(self):
        import _complete_registry_runtime as original_fixture
        for cleanup, cancellation, stop_fails in ((True, False, False), (False, False, False),
                                                  (False, True, False), (True, False, True)):
            with self.subTest(cleanup=cleanup, cancellation=cancellation, stop_fails=stop_fails), \
                    tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                args = SimpleNamespace(run_root=root/'fresh', report=root/'report.json',
                    bounded_policy=root/'policy.json', registry_limits=root/'registry.json',
                    build_limits=root/'build.json', tools=root/'tools')
                services = Mock(center_config={'deployment_id': 'test-deployment'},
                                manifest_sha256='a'*64)
                services.execution.return_value = ({'same_original_execution': True}, {'center': {'pid': 123}})
                if stop_fails:
                    services.stop_all.side_effect = RuntimeError('original owned exit unknown')
                deployment = SimpleNamespace(services=services, settings=[{}, dict(
                    kernel='original-kernel', rootfs='original-rootfs')], build_config=root/'generated.json')
                report = dict(passed=cleanup, guest_passed=True, cleanup={'passed': cleanup})
                with ExitStack() as stack:
                    stack.enter_context(patch.dict(probe.sys.modules, {
                        'template_build': SimpleNamespace(BUILDKIT_HASHES={})}))
                    stack.enter_context(patch.object(probe.sys, 'platform', 'linux'))
                    stack.enter_context(patch.object(probe.os, 'geteuid', return_value=0))
                    stack.enter_context(patch.object(probe.importlib.metadata, 'version', return_value='2.51.0'))
                    stack.enter_context(patch.object(probe.host, 'source_revision', return_value={'revision': 'b'*40}))
                    stack.enter_context(patch.object(probe.host, 'trusted_path', side_effect=lambda value: value))
                    stack.enter_context(patch.object(probe, 'digest', return_value=
                        '8e0e62a497fcdb8048d18aa927a139613176ba0531f412bc541044e28f9856bd'))
                    stack.enter_context(patch.object(probe, 'private_json', return_value={}))
                    prepared = stack.enter_context(patch.object(original_fixture,
                        'prepare_registry_runtime', return_value=deployment))
                    stack.enter_context(patch('server_process_crash.free_loopback_port',
                        side_effect=[4100, 4101, 4102]))
                    stack.enter_context(patch.object(probe, 'configuration', return_value={}))
                    stack.enter_context(patch.object(probe, 'validate_configuration'))
                    stack.enter_context(patch.object(probe, 'fixed_artifacts', return_value={
                        'kernel': probe.digest('kernel'), 'rootfs': probe.digest('rootfs')}))
                    stack.enter_context(patch.object(probe, 'host_uid_range', return_value=range(1000, 1002)))
                    stack.enter_context(patch('sdk_c02_acceptance.processes_in_uid_range', return_value=[]))
                    stack.enter_context(patch.object(probe, 'Backend'))
                    stack.enter_context(patch.object(probe, 'Observations'))
                    supervise = stack.enter_context(patch.object(probe, 'supervise',
                        side_effect=KeyboardInterrupt() if cancellation else None, return_value=report))
                    observed = probe.exercise(args)
                self.assertEqual(services.start.call_count, 1, observed)
                services.start.assert_called_once_with()
                services.wait_ready.assert_called_once_with()
                services.stop_all.assert_called_once_with()
                services.cleanup_staging.assert_not_called()
                prepared.assert_called_once()
                supervise.assert_called_once()
                self.assertEqual(observed['passed'], cleanup and not cancellation and not stop_fails)
                self.assertTrue(args.report.is_file())
                self.assertTrue(args.report.with_suffix('.execution.json').is_file())
                self.assertEqual(json.loads(args.report.read_text()), observed)
                if cancellation:
                    self.assertEqual(observed['error_type'], 'KeyboardInterrupt')
                if stop_fails:
                    self.assertEqual(observed['service_exit'], 'unknown')


if __name__ == '__main__':
    unittest.main()
