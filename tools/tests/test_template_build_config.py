import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.template_build_config import validate_capacity, MIB, UPLOAD_ENVELOPE_BYTES
from tools.template_build_service import configuration
from tools.template_inputs import InvalidTemplate, platform_dns_servers
from tools.template_worker import install_platform, validate_request

GIB = 1024 * MIB


def inputs():
    # Synthetic necessary-condition fixture, not a recommended host profile.
    compute = {'max_vcpus_global': 8, 'max_vcpus_per_tenant': 8,
               'max_memory_bytes_global': 18 * GIB, 'max_memory_bytes_per_tenant': 18 * GIB}
    runtime = {'max_timeout_secs': 86400, 'max_lifetime_secs': 86400,
               'max_disk_bytes_global': 80 * GIB, 'max_disk_bytes_per_tenant': 80 * GIB,
               'cleanup_reserve_bytes': 10 * GIB, 'max_active_global': 2,
               'max_active_per_tenant': 2, 'max_vm_id': 99,
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
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            runtime, builder, limits, base = inputs()
            for pool, index in ((runtime, 0), (builder, 1)):
                directory = root / str(index)
                directory.mkdir(mode=0o700)
                pool.update(namespace=str(index), database=str(directory / 'db'), node_state_dir=str(directory),
                            chroot_base=str(directory), bind=f'127.0.0.1:{3100 + index}',
                            uid_base=1000 + index * 100, credentials=[{'tenant': 1, 'secret': str(index)}],
                            host_resources={'cgroup_parent': str(directory)})
            registry = root / 'registry'
            for directory in (registry, registry / 'inbox', registry / 'artifacts', root / 'builds'):
                directory.mkdir(mode=0o700)
            runtime['template_registry']['root'] = str(registry)
            builder['internal_build_vms'] = True
            kernel = root / 'kernel'
            kernel.write_bytes(b'kernel')
            runtime['kernel'] = str(kernel)
            disk = root / 'builder.ext4'
            with disk.open('wb') as output:
                output.truncate(32 * GIB)  # sparse synthetic input; no guest/mount
            base.update(template_id='actual-base', owner=None)
            base['rootfs']['path'] = str(disk)
            builder['templates'].update(templates=[base], aliases={'base': 'actual-base'})
            builder['rootfs'] = str(kernel)  # legacy field must not undercharge
            files = []
            for pool, name in ((runtime, 'runtime.json'), (builder, 'builder.json')):
                path = root / name
                path.write_text(json.dumps(pool))
                path.chmod(0o600)
                files.append(str(path))
            tool_names = ('buildctl', 'buildkitd', 'buildkit-runc', 'umoci', 'envd', 'busybox', 'regctl')
            cfg = {'root': str(root / 'builds'), 'bind': '127.0.0.1:3102',
                   'runtime_config': files[0], 'builder_config': files[1],
                   'tools': {name: {'path': str(kernel), 'sha256': hashlib.sha256(b'kernel').hexdigest()} for name in tool_names},
                   'limits': limits, 'dns_servers': ['1.1.1.1']}
            path = root / 'service.json'
            path.write_text(json.dumps(cfg))
            path.chmod(0o600)
            self.assertEqual(configuration(path)['limits']['builder_disk_bytes'], 32 * GIB)
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


class DnsTests(unittest.TestCase):
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
