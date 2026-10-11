import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import subprocess
import sys

from tools.template_build_config import validate_capacity, MIB, UPLOAD_ENVELOPE_BYTES
from tools.template_build_config import bounded_data_route_limits, host_uid_range, validate_pool_resource_ranges
from tools.template_build_service import configuration
from tools.template_inputs import InvalidTemplate, platform_dns_servers
from tools.template_worker import install_platform, validate_request

GIB = 1024 * MIB


class ConfigurationDependencyTests(unittest.TestCase):
    def test_service_and_client_cli_help_remain_available_in_both_entry_modes(self):
        root = Path(__file__).resolve().parents[2]
        for module in ("template_build_service", "template_build_client"):
            for arguments in ([str(root / "tools" / (module + ".py")), "--help"],
                              ["-m", "tools." + module, "--help"]):
                with self.subTest(arguments=arguments):
                    result = subprocess.run([sys.executable, *arguments], cwd=root,
                                            capture_output=True, text=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("usage:", result.stdout)

    def test_shared_checks_work_without_loading_http_service_in_both_import_modes(self):
        root = Path(__file__).resolve().parents[2]
        for module, search_root in (("tools.template_build_config", root),
                                    ("template_build_config", root / "tools")):
            code = """
import builtins, importlib, json, os, pathlib, sys, tempfile
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[2])
original = builtins.__import__
def guarded(name, *args, **kwargs):
    if 'template_build_service' in name or name == 'http.server':
        raise AssertionError('shared configuration imported HTTP service')
    return original(name, *args, **kwargs)
builtins.__import__ = guarded
config = importlib.import_module(sys.argv[1])
with tempfile.TemporaryDirectory() as temporary:
    path = pathlib.Path(temporary).resolve() / 'policy.json'
    path.write_text(json.dumps({'runtime': {}, 'builder': {}, 'public': {}}))
    os.chmod(path, 0o600)
    assert config.protected_json(path) == {'runtime': {}, 'builder': {}, 'public': {}}
    try:
        config.acceptance_build_policy(path)
    except ValueError as error:
        assert 'explicit bounded pool policy' in str(error)
    else:
        raise AssertionError('incomplete policy accepted')
settings = json.loads(sys.argv[3])
assert list(config.host_uid_range(settings)) == [1000, 1001]
builder = json.loads(sys.argv[3])
builder['uid_base'] = builder['bounded_host']['uid_base'] = 2000
builder['bounded_host']['uplink_base'] = 2
config.validate_pool_resource_ranges(settings, builder)
assert not any('template_build_service' in name for name in sys.modules)
"""
            with self.subTest(module=module):
                result = subprocess.run([sys.executable, "-I", "-c", code, module,
                                         str(search_root), json.dumps(HostUidRangeTests().settings())],
                                        capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, 0, result.stderr)


def transfer_settings():
    # Explicit synthetic policy, not deployment defaults.
    return {'file_upload_bytes': 1024, 'file_download_bytes': 1024,
            'command_input_bytes': 1024,
            'rpc_request_bytes': 1024, 'rpc_response_bytes': 1024,
            'max_transfers': 8, 'max_transfers_per_vm': 4, 'control_reserve': 1,
            'request_idle_seconds': 5, 'file_idle_seconds': 5, 'write_idle_seconds': 5}


class HostUidRangeTests(unittest.TestCase):
    def settings(self):
        # Explicit synthetic limits, never deployment defaults.
        return {'uid_base': 1000, 'data_transfer': transfer_settings(), 'bounded_host': {
            'uid_base': 1000, 'uplink_base': 0, 'maintenance': {'max_retained_results': 10,
                'max_host_slots': 2, 'windows': {'max_open_windows': 2, 'max_duration_ms': 100}}}}

    def test_bounded_route_capacity_uses_explicit_per_vm_cap_including_control_reserve(self):
        settings = self.settings()
        self.assertEqual(bounded_data_route_limits(settings), {
            'max_data_routes_per_slot': 4, 'max_data_routes_global': 8, 'data_route_control_reserve': 1})
        settings['data_transfer']['max_transfers_per_vm'] = 5
        self.assertEqual(bounded_data_route_limits(settings)['max_data_routes_per_slot'], 5)
        settings['data_transfer']['max_transfers'] = 16
        self.assertEqual(bounded_data_route_limits(settings), {
            'max_data_routes_per_slot': 5, 'max_data_routes_global': 16, 'data_route_control_reserve': 1})
        settings['data_transfer']['control_reserve'] = 2
        self.assertEqual(bounded_data_route_limits(settings), {
            'max_data_routes_per_slot': 5, 'max_data_routes_global': 16, 'data_route_control_reserve': 2})
        for field in ('max_data_routes_per_slot', 'max_data_routes_global', 'data_route_control_reserve'):
            duplicated = copy.deepcopy(settings)
            duplicated['bounded_host'][field] = 5
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'bounded host config'):
                host_uid_range(duplicated)

    def test_bounded_route_capacity_rejects_missing_untyped_and_inconsistent_budgets(self):
        for invalid in (None, {}, [], True):
            settings = self.settings()
            settings['data_transfer'] = invalid
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                host_uid_range(settings)
        settings = self.settings()
        del settings['data_transfer']
        with self.assertRaises(ValueError):
            host_uid_range(settings)
        settings['bounded_host'] = None
        settings['max_vm_id'] = 99
        with self.assertRaises(ValueError):
            host_uid_range(settings)
        for field in transfer_settings():
            for invalid in (0, -1, True, None, '2', 1.5, 2**64):
                settings = self.settings()
                settings['data_transfer'][field] = invalid
                with self.subTest(field=field, value=invalid), self.assertRaises(ValueError):
                    host_uid_range(settings)
            settings = self.settings()
            del settings['data_transfer'][field]
            with self.subTest(missing=field), self.assertRaises(ValueError):
                host_uid_range(settings)
        for field, value in (('control_reserve', 4), ('max_transfers', 3),
                             ('max_transfers_per_vm', 9), ('rpc_request_bytes', 8 * MIB + 1),
                             ('request_idle_seconds', 2**63)):
            settings = self.settings()
            settings['data_transfer'][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                host_uid_range(settings)

    def test_bounded_span_is_physical_count_and_retired_mapping_is_rejected(self):
        settings = self.settings()
        self.assertEqual(host_uid_range(settings), range(1000, 1002))
        settings['bounded_host']['maintenance']['max_host_slots'] = 3
        self.assertEqual(host_uid_range(settings), range(1000, 1003))
        for invalid in ({'uid_base': 1000, 'max_vm_id': 99},
                        settings | {'max_vm_id': 0},
                        settings | {'bounded_host': None}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                host_uid_range(invalid)

    def test_last_usable_uid_and_adjacent_pools_are_exact(self):
        settings = self.settings()
        settings['uid_base'] = settings['bounded_host']['uid_base'] = 2**32 - 3
        self.assertEqual(host_uid_range(settings), range(2**32 - 3, 2**32 - 1))
        settings['bounded_host']['maintenance']['max_host_slots'] = 3
        with self.assertRaises(ValueError):
            host_uid_range(settings)
        runtime = self.settings()
        builder = self.settings()
        builder['uid_base'] = builder['bounded_host']['uid_base'] = 1002
        r_range, b_range = host_uid_range(runtime), host_uid_range(builder)
        self.assertEqual(r_range.stop, b_range.start)

    def test_all_bounded_limits_are_explicit_and_strictly_typed(self):
        valid = self.settings()
        paths = [('uid_base',), ('bounded_host', 'uid_base'),
                 ('bounded_host', 'maintenance', 'max_retained_results'),
                 ('bounded_host', 'maintenance', 'max_host_slots'),
                 ('bounded_host', 'maintenance', 'windows', 'max_open_windows'),
                 ('bounded_host', 'maintenance', 'windows', 'max_duration_ms')]
        for path in paths:
            for value in (0, -1, True, False, '2', 1.5, None, 2**64):
                settings = copy.deepcopy(valid)
                target = settings
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                with self.subTest(path=path, value=value), self.assertRaises(ValueError):
                    host_uid_range(settings)
            settings = copy.deepcopy(valid)
            target = settings
            for key in path[:-1]:
                target = target[key]
            del target[path[-1]]
            with self.subTest(missing=path), self.assertRaises(ValueError):
                host_uid_range(settings)
        for path in [('bounded_host',), ('bounded_host', 'maintenance'),
                     ('bounded_host', 'maintenance', 'windows')]:
            settings = copy.deepcopy(valid)
            target = settings
            for key in path:
                target = target[key]
            target['misspelled'] = 1
            with self.subTest(unknown=path), self.assertRaises(ValueError):
                host_uid_range(settings)
        settings = self.settings()
        settings['bounded_host']['uid_base'] += 1
        with self.assertRaisesRegex(ValueError, 'differs'):
            host_uid_range(settings)
        settings = self.settings()
        settings['bounded_host']['maintenance']['max_host_slots'] = 1_048_577
        with self.assertRaises(ValueError):
            host_uid_range(settings)
        for invalid in (-1, True, '0', 1.5, None, 1_048_575, 2**64):
            settings = self.settings()
            settings['bounded_host']['uplink_base'] = invalid
            with self.subTest(uplink_base=invalid), self.assertRaises(ValueError):
                host_uid_range(settings)
        settings = self.settings()
        del settings['bounded_host']['uplink_base']
        with self.assertRaises(ValueError):
            host_uid_range(settings)

    def test_public_pools_need_disjoint_addresses_even_with_distinct_namespaces(self):
        runtime, builder = self.settings(), self.settings()
        runtime.update(namespace='runtime', public_ipv4_enabled=True)
        builder.update(namespace='builder', public_ipv4_enabled=True, uid_base=1002)
        builder['bounded_host']['uid_base'] = 1002
        with self.assertRaisesRegex(ValueError, 'uplink pools overlap'):
            validate_pool_resource_ranges(runtime, builder)
        builder['bounded_host']['uplink_base'] = 2
        validate_pool_resource_ranges(runtime, builder)
        builder['bounded_host']['uplink_base'] = 1
        with self.assertRaisesRegex(ValueError, 'uplink pools overlap'):
            validate_pool_resource_ranges(runtime, builder)
        runtime['public_ipv4_enabled'] = False
        with self.assertRaisesRegex(ValueError, 'uplink pools overlap'):
            validate_pool_resource_ranges(runtime, builder)
        builder['public_ipv4_enabled'] = False
        with self.assertRaisesRegex(ValueError, 'uplink pools overlap'):
            validate_pool_resource_ranges(runtime, builder)
        builder['bounded_host']['uplink_base'] = 2
        validate_pool_resource_ranges(runtime, builder)
        runtime['public_ipv4_enabled'] = 'false'
        with self.assertRaisesRegex(ValueError, 'enable flag'):
            validate_pool_resource_ranges(runtime, builder)

    def test_legacy_and_mixed_pools_are_not_formal_configurations(self):
        runtime = self.settings()
        builder = self.settings()
        builder['uid_base'] = builder['bounded_host']['uid_base'] = 1100
        builder['bounded_host']['uplink_base'] = 2
        validate_pool_resource_ranges(runtime, builder)
        legacy = {'uid_base': 1000, 'max_vm_id': 99, 'public_ipv4_enabled': True}
        for pools in ((legacy, builder), (runtime, legacy), (legacy, legacy)):
            with self.subTest(pools=pools), self.assertRaises(ValueError):
                validate_pool_resource_ranges(*pools)


def inputs():
    # Synthetic necessary-condition fixture, not a recommended host profile.
    compute = {'max_vcpus_global': 8, 'max_vcpus_per_tenant': 8,
               'max_memory_bytes_global': 18 * GIB, 'max_memory_bytes_per_tenant': 18 * GIB}
    runtime = {'max_timeout_secs': 86400, 'max_lifetime_secs': 86400,
               'max_disk_bytes_global': 80 * GIB, 'max_disk_bytes_per_tenant': 80 * GIB,
               'cleanup_reserve_bytes': 10 * GIB, 'max_active_global': 2,
               'max_active_per_tenant': 2,
               'templates': {'compute_limits': compute},
               'template_registry': {'validation_timeout_secs': 45,
                   'max_artifact_bytes_global': 300 * GIB, 'max_artifact_bytes_per_tenant': 300 * GIB}}
    builder = copy.deepcopy(runtime)
    builder.pop('template_registry')
    builder.update(max_timeout_secs=660, max_lifetime_secs=660, public_ipv4_enabled=True,
                   data_transfer={'file_upload_bytes': GIB, 'file_download_bytes': 10 * GIB})
    limits = {'max_cpu': 4, 'max_memory_mib': 8192, 'max_disk_mib': 10240,
              'timeout_seconds': 600, 'max_jobs': 100, 'max_jobs_per_tenant': 100,
              'max_pending': 1, 'max_pending_per_tenant': 1, 'workspace_bytes': 200 * GIB,
              'workspace_bytes_per_tenant': 200 * GIB, 'max_input_bytes': 512 * MIB,
              'max_expanded_bytes': 4 * GIB, 'max_entries': 200000, 'max_log_bytes': 2 * MIB}
    base = {'rootfs': {'size_bytes': 32 * GIB}, 'resources': {
        'cpu_count': 2, 'host_memory_bytes': 2304 * MIB, 'disk_bytes': 32 * GIB}}
    return runtime, builder, limits, base



class CapacityTests(unittest.TestCase):
    def check(self, args):
        return validate_capacity(*args, kernel_bytes=64 * MIB, largest_tool_bytes=128 * MIB)

    def test_capacity_covers_one_maximum_candidate_and_actual_builder(self):
        self.assertEqual(self.check(inputs()), 32 * GIB)

    def test_both_lifetime_limits_and_publication_lifetime_are_checked(self):
        for pool, field, lower in ((1, 'max_timeout_secs', 659), (1, 'max_lifetime_secs', 659),
                                   (0, 'max_lifetime_secs', 44), (0, 'max_timeout_secs', 44)):
            args = inputs()
            args[pool][field] = lower
            with self.subTest(pool=pool, field=field), self.assertRaisesRegex(ValueError, 'lifetime'):
                self.check(args)
        args = inputs()
        del args[1]['max_lifetime_secs']
        self.check(args)  # same documented default as the controller
        args[1]["max_lifetime_secs"] = None
        self.check(args)

    def test_disk_and_transfer_exact_boundaries(self):
        args = inputs()
        runtime, builder, limits, base = args
        required = limits['max_disk_mib'] * MIB + limits['max_expanded_bytes'] + 2 * limits['max_input_bytes'] + 64 * MIB
        base['resources']['disk_bytes'] = base['rootfs']['size_bytes'] = required
        builder['data_transfer']['file_upload_bytes'] = limits['max_input_bytes'] + UPLOAD_ENVELOPE_BYTES
        builder['data_transfer']['file_download_bytes'] = limits['max_disk_mib'] * MIB
        self.check(args)
        for field in ('file_upload_bytes', 'file_download_bytes'):
            broken = copy.deepcopy(args)
            broken[1]['data_transfer'][field] -= 1
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'budget'):
                self.check(broken)
        base['resources']['disk_bytes'] -= 1
        base['rootfs']['size_bytes'] -= 1
        with self.assertRaisesRegex(ValueError, 'builder disk'):
            self.check(args)

    def test_runtime_publication_and_compute_are_not_guessed(self):
        for path, value in ((('templates', 'compute_limits', 'max_vcpus_per_tenant'), 3),
                            (('templates', 'compute_limits', 'max_memory_bytes_global'), 8192 * MIB),
                            (('max_disk_bytes_global',), 10 * GIB),
                            (('template_registry', 'max_artifact_bytes_per_tenant'), 10 * GIB)):
            args = inputs()
            target = args[0]
            for part in path[:-1]:
                target = target[part]
            target[path[-1]] = value
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'budget'):
                self.check(args)

    def test_configuration_charges_resolved_base_not_legacy_rootfs(self):
        from tools.tests.test_bounded_acceptance import service_fixture
        from tools.tests.test_template_physical_storage import activate_for_test
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path, _, profile, _ = service_fixture(root)
            Path(profile['coordination_file']).touch(mode=0o600)
            activate_for_test(profile)
            cfg = json.loads(path.read_text())
            builder_path = Path(cfg['builder_config'])
            builder = json.loads(builder_path.read_text())
            builder['rootfs'] = str(root / 'kernel')  # Never undercharge its bound base.
            builder_path.write_text(json.dumps(builder))
            self.assertEqual(configuration(path)['limits']['builder_disk_bytes'], 32 * GIB)
            for old in (None, 0, 2**64 - 1):
                builder['max_vm_id'] = old
                builder_path.write_text(json.dumps(builder))
                with self.subTest(old=old), self.assertRaisesRegex(ValueError, 'retired max_vm_id'):
                    configuration(path)
            builder.pop('max_vm_id')
            builder['bounded_host']['uplink_base'] = 0
            builder_path.write_text(json.dumps(builder))
            with self.assertRaisesRegex(ValueError, 'uplink pools overlap'):
                configuration(path)
            builder['bounded_host']['uplink_base'] = 2
            builder_path.write_text(json.dumps(builder))
            original_tools = dict(cfg['tools'])
            for names, accepted in ((('igzip', 'libisal.so.2'), True), (('igzip',), False),
                                    (('libisal.so.2',), False), (('unpigz',), False)):
                cfg['tools'] = original_tools | {name: original_tools['buildctl'] for name in names}
                path.write_text(json.dumps(cfg))
                if accepted:
                    self.assertEqual(set(configuration(path)['tools']), set(cfg['tools']))
                else:
                    with self.assertRaisesRegex(ValueError, 'tool profile'):
                        configuration(path)
            cfg['tools'] = original_tools
            cfg['dns_servers'] = [134744072]
            path.write_text(json.dumps(cfg))
            with self.assertRaisesRegex(ValueError, 'strings'):
                configuration(path)

    def test_formal_service_rejects_removed_or_missing_build_configuration(self):
        from tools.tests.test_bounded_acceptance import service_fixture
        with tempfile.TemporaryDirectory() as temporary:
            path, _, _, _ = service_fixture(Path(temporary).resolve())
            original = json.loads(path.read_text())
            for changed in (original | {'migrate_legacy_builds': False},
                            {key: value for key, value in original.items() if key != 'bounded_builds'},
                            original | {'bounded_builds': None}):
                path.write_text(json.dumps(changed))
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    configuration(path, require_physical_lease=False)


class DnsTests(unittest.TestCase):
    def test_mirror_worker_setting_is_typed_and_only_for_public_oci(self):
        request = {'kind': 'oci-reference', 'disk_mib': 1024, 'min_free_mib': 64,
                   'max_input_bytes': 1024, 'max_expanded_bytes': 10240,
                   'max_entries': 10, 'timeout_seconds': 30, 'max_log_bytes': 1024,
                   'docker_hub_mirror': 'docker.1ms.run'}
        validate_request(request)
        for kind in ('dockerfile', 'oci'):
            with self.assertRaises(InvalidTemplate):
                validate_request(request | {'kind': kind})
        for mirror in (None, 'https://docker.1ms.run', '10.0.0.1', 'host.local', 'docker.1ms.run/path'):
            with self.subTest(mirror=mirror), self.assertRaises(InvalidTemplate):
                validate_request(request | {'docker_hub_mirror': mirror})

    def test_same_typed_public_unicast_contract_for_service_and_worker(self):
        request = {'kind': 'dockerfile', 'disk_mib': 1024, 'min_free_mib': 64,
                   'max_input_bytes': 1024, 'max_expanded_bytes': 10240,
                   'max_entries': 10, 'timeout_seconds': 30, 'max_log_bytes': 1024}
        for value in (['1.1.1.1'], ['1.1.1.1', '8.8.8.8']):
            self.assertEqual(platform_dns_servers(value), value)
            validate_request(request | {'dns_servers': value})
        for value in (None, [], '1.1.1.1', [134744072], [True], ['224.0.0.1'], ['240.0.0.1'],
                      ['127.0.0.53'], ['10.0.0.1'], ['169.254.169.254'], ['100.64.0.1'],
                      ['::1'], ['example.com'], ['1.1.1.1\nnameserver 8.8.8.8'], ['1.1.1.1'] * 3):
            with self.subTest(value=value):
                with self.assertRaises(InvalidTemplate):
                    platform_dns_servers(value)
                with self.assertRaises(InvalidTemplate):
                    validate_request(request | {'dns_servers': value})

    def test_candidate_resolver_replaces_link_without_touching_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'guest'
            (root / 'etc').mkdir(parents=True)
            (root / 'etc/os-release').write_text('ID=debian\n')
            (root / 'etc/passwd').write_text('root:x:0:0:root:/root:/bin/sh\n')
            canary = Path(temporary) / 'host-canary'
            canary.write_text('unchanged')
            (root / 'etc/resolv.conf').symlink_to(canary)
            binary = Path(temporary) / 'binary'
            binary.write_bytes(b'synthetic')
            dns = ['1.1.1.1', '8.8.8.8']
            install_platform(root, binary, binary, platform_dns_servers(dns))
            self.assertFalse((root / 'etc/resolv.conf').is_symlink())
            self.assertEqual((root / 'etc/resolv.conf').read_text(), 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n')
            self.assertEqual(canary.read_text(), 'unchanged')


if __name__ == '__main__':
    unittest.main()


class ExplicitAcceptanceModeTests(unittest.TestCase):
    def test_mode_is_explicit_and_policy_shape_cannot_inject_pool_identity(self):
        import json
        from pathlib import Path
        import tempfile
        from tools.template_build_config import acceptance_build_policy
        with self.assertRaisesRegex(ValueError, 'complete bounded policy'):
            acceptance_build_policy(None)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary).resolve() / 'policy.json'
            path.write_text(json.dumps({'runtime': {'database': 'replace'}, 'builder': {}, 'public': {}}))
            path.chmod(0o600)
            with self.assertRaisesRegex(ValueError, 'explicit bounded pool policy'):
                acceptance_build_policy(path)
            with self.assertRaises(TypeError):
                acceptance_build_policy(path, legacy_unbounded=True)

    def test_policy_passes_explicit_sections_and_never_changes_existing_identity(self):
        import copy
        from tools.template_build_config import apply_acceptance_build_policy
        from unittest.mock import patch
        runtime = {'namespace': 'runtime', 'database': '/runtime/db'}
        builder = {'namespace': 'builder', 'database': '/builder/db'}
        build = {'limits': {'workspace_bytes': 100, 'workspace_bytes_per_tenant': 50}}
        policy = {'runtime': {'bounded_host': {'uid_base': 1000}, 'maintenance': {'capacity': {}}},
                  'builder': {'bounded_host': {'uid_base': 2000}, 'maintenance': {'capacity': {}},
                              'build_retry_windows': {'window_duration_ms': 1000, 'max_anchors': 2}},
                  'public': {'bounded_builds': {'namespace': 17}, 'workspace_bytes': 200, 'workspace_bytes_per_tenant': 100}}
        before = copy.deepcopy((runtime, builder, build))
        with self.assertRaisesRegex(ValueError, 'invalid bounded acceptance policy'):
            apply_acceptance_build_policy(runtime, builder, build, None)
        self.assertEqual((runtime, builder, build), before)
        for pool, name in enumerate(('runtime', 'builder')):
            legacy = copy.deepcopy((runtime, builder, build, policy))
            legacy[pool]['max_vm_id'] = None
            original = copy.deepcopy(legacy)
            with self.subTest(pool=name), self.assertRaisesRegex(ValueError, 'retired max_vm_id'):
                apply_acceptance_build_policy(*legacy)
            self.assertEqual(legacy, original)
        with patch('tools.template_build_config.validate_pool_resource_ranges') as ranges, \
                patch('tools.template_build_store.bounded_configuration'), \
                patch('tools.template_physical_storage.validate_configuration') as physical:
            self.assertEqual(apply_acceptance_build_policy(runtime, builder, build, policy), 'windowed')
            ranges.assert_called_once_with(runtime, builder)
            physical.assert_called_once_with(runtime, builder, build)
        self.assertEqual(runtime['namespace'], 'runtime')
        self.assertEqual(builder['database'], '/builder/db')
        self.assertEqual(builder['build_retry_windows'], policy['builder']['build_retry_windows'])
        self.assertEqual(build['limits']['workspace_bytes'], 200)
        self.assertEqual(build['bounded_builds'], {'namespace': 17})

    def test_generator_rejects_mismatched_physical_budget_without_activation(self):
        import copy
        from unittest.mock import patch
        from tools.template_build_config import apply_acceptance_build_policy
        from tools.tests.test_template_physical_storage import configuration as physical_fixture
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime, builder, build, profile = physical_fixture(root)
            build['limits']['workspace_bytes_per_tenant'] = 1000
            policy = {
                'runtime': {key: copy.deepcopy(runtime[key]) for key in ('bounded_host', 'maintenance')},
                'builder': {key: copy.deepcopy(builder[key]) for key in ('bounded_host', 'maintenance')},
                'public': {'bounded_builds': copy.deepcopy(build['bounded_builds']),
                           'workspace_bytes': 2000, 'workspace_bytes_per_tenant': 1000},
            }
            policy['builder']['build_retry_windows'] = {'window_duration_ms': 1000, 'max_anchors': 2}
            runtime['max_disk_bytes_global'] += 1
            before = copy.deepcopy((runtime, builder, build))
            with patch('tools.template_build_config.validate_pool_resource_ranges'), \
                    patch('tools.template_build_store.bounded_configuration'):
                with self.assertRaisesRegex(ValueError, 'pool budget'):
                    apply_acceptance_build_policy(runtime, builder, build, policy)
            self.assertEqual((runtime, builder, build), before)
            self.assertEqual(Path(profile['coordination_file']).read_bytes(), b'')
            self.assertEqual(list((root / 'public').iterdir()), [])

    def test_generator_rejects_missing_physical_profile_without_mutating_inputs(self):
        import copy
        from unittest.mock import patch
        from tools.template_build_config import apply_acceptance_build_policy
        from tools.tests.test_template_physical_storage import configuration as physical_fixture
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime, builder, build, profile = physical_fixture(root)
            build['limits']['workspace_bytes_per_tenant'] = 1000
            for pool in (runtime, builder):
                pool['bounded_host'].pop('physical_storage')
                pool['maintenance'].pop('capacity')
            policy = {
                'runtime': {key: copy.deepcopy(runtime[key]) for key in ('bounded_host', 'maintenance')},
                'builder': {key: copy.deepcopy(builder[key]) for key in ('bounded_host', 'maintenance')},
                'public': {'bounded_builds': copy.deepcopy(build['bounded_builds']),
                           'workspace_bytes': 2000, 'workspace_bytes_per_tenant': 1000},
            }
            policy['builder']['build_retry_windows'] = {'window_duration_ms': 1000, 'max_anchors': 2}
            before = copy.deepcopy((runtime, builder, build))
            with patch('tools.template_build_config.validate_pool_resource_ranges'), \
                    patch('tools.template_build_store.bounded_configuration'), \
                    self.assertRaisesRegex(ValueError, 'identical explicit'):
                apply_acceptance_build_policy(runtime, builder, build, policy)
            self.assertEqual((runtime, builder, build), before)
            self.assertEqual(Path(profile['coordination_file']).read_bytes(), b'')
            self.assertEqual(list((root / 'public').iterdir()), [])
