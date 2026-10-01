import copy
import hashlib
import json
from pathlib import Path
import tempfile
import threading
import unittest

from tools.generic_capacity import GIB, MIB, plan
from tools.template_acceptance import budgets, check_headroom, load, PROFILE, RUNTIME_PROFILE
from tools.template_build_client import BuildClient
from tools.template_build_service import BuildHTTP, Supervisor, configuration
from tools.template_build_store import BuildStore
from tools.tests.test_template_build_service import FakeBackend, PowerLoss


def inputs(count=4):
    entries = [{'name': str(i), 'requirements': {'cpus': 4, 'memory_mib': 8192,
                'disk_mib': 10240, 'build_timeout_sec': 600}} for i in range(count)]
    return plan(entries, 1, 32 * GIB, 64 * MIB)


def configured(root, deployment, **extra_config):
    """Real protected config reader, synthetic sparse disk; never launch it."""
    kernel = root / 'kernel'
    kernel.write_bytes(b'kernel')
    disk = root / 'base.ext4'
    with disk.open('wb') as output:
        output.truncate(32 * GIB)
    pool_files = []
    for i, kind in enumerate(('runtime', 'builder')):
        directory = root / str(i)
        directory.mkdir(mode=0o700)
        pool = copy.deepcopy(deployment[kind])
        shape = deployment['builder_shape'] if i else {'cpu_count': 1, 'memory_mib': 2048, 'host_memory_bytes': 2304 * MIB}
        base = {'template_id': 'base', 'owner': None, 'rootfs': {'path': str(disk), 'size_bytes': 32 * GIB},
                'resources': shape | {'disk_bytes': 32 * GIB}}
        pool.update(namespace=str(i), database=str(directory / 'db'), node_state_dir=str(directory),
                    chroot_base=str(directory), bind=f'127.0.0.1:{3100 + i}', uid_base=1000 + 100 * i,
                    max_vm_id=99, credentials=[{'tenant': 1, 'secret': f'pool-{i}'}], kernel=str(kernel),
                    rootfs=str(disk), host_resources={'cgroup_parent': str(directory)},
                    public_ipv4_enabled=True, templates={'templates': [base], 'aliases': {'base': 'base'},
                        'compute_limits': deployment[kind + '_compute']})
        if i:
            pool['internal_build_vms'] = True
        else:
            registry = root / 'registry'
            for path in (registry, registry / 'inbox', registry / 'artifacts'):
                path.mkdir(mode=0o700)
            pool['template_registry'] = deployment['registry'] | {'root': str(registry)}
        path = root / f'{kind}.json'
        path.write_text(json.dumps(pool))
        path.chmod(0o600)
        pool_files.append(path)
    (root / 'builds').mkdir(mode=0o700)
    tools = ('buildctl', 'buildkitd', 'buildkit-runc', 'umoci', 'envd', 'busybox', 'regctl')
    config = {'root': str(root / 'builds'), 'bind': '127.0.0.1:3102',
              'runtime_config': str(pool_files[0]), 'builder_config': str(pool_files[1]),
              'tools': {name: {'path': str(kernel), 'sha256': hashlib.sha256(b'kernel').hexdigest()} for name in tools},
              'limits': deployment['limits'], 'dns_servers': ['1.1.1.1']}
    config.update(extra_config)
    path = root / 'build.json'
    path.write_text(json.dumps(config))
    path.chmod(0o600)
    return configuration(path)


class AcceptanceConfigTests(unittest.TestCase):
    def test_optional_mirror_is_in_protected_configuration_and_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            cfg = configured(root, budgets(inputs(), 32 * GIB, 64 * MIB),
                             docker_hub_mirror='docker.1ms.run')
            self.assertEqual(cfg['docker_hub_mirror'], 'docker.1ms.run')
            original = cfg.pop('binding')
            self.assertEqual(original, hashlib.sha256(json.dumps(cfg, sort_keys=True).encode()).hexdigest())
            del cfg['docker_hub_mirror']
            self.assertNotEqual(original, hashlib.sha256(json.dumps(cfg, sort_keys=True).encode()).hexdigest())

    def test_explicit_small_host_profile_preserves_four_instance_cohort_and_headroom(self):
        profile = load(PROFILE.with_name('template-service-budgets-16g.json'))
        entries = [{'name': str(i), 'requirements': {'cpus': 1, 'memory_mib': 2048,
                    'disk_mib': 10240, 'build_timeout_sec': 600}} for i in range(4)]
        capacity = plan(entries, 1, 32 * GIB, 64 * MIB)
        deployment = budgets(capacity, 32 * GIB, 64 * MIB, profile=profile)
        self.assertEqual(deployment['runtime']['max_active_global'], 4)
        self.assertEqual(deployment['runtime_compute']['max_memory_bytes_global'], 4 * 2304 * MIB)
        self.assertEqual(deployment['builder_shape']['memory_mib'], 2048)
        self.assertEqual(deployment['host']['minimum_available_memory_bytes'], 13568 * MIB)
        self.assertEqual(deployment['host']['memory_headroom_bytes'], GIB)
        self.assertEqual(deployment['service_cgroup']['memory_bytes'], GIB)
        with tempfile.TemporaryDirectory() as temporary:
            configured(Path(temporary).resolve(), deployment)
        exact = dict(cpus=8, available_memory=13568 * MIB,
                     free_disk=deployment['host']['minimum_free_disk_bytes'])
        check_headroom(deployment, **exact)
        with self.assertRaisesRegex(ValueError, 'memory'):
            check_headroom(deployment, **(exact | {'available_memory': exact['available_memory'] - 1}))
        # Keep original defaults and shape admission: requested memory beyond
        # the runtime quota must not be silently reduced to fit this host.
        self.assertEqual(load()['builder']['memory_mib'], 8192)
        self.assertNotEqual(deployment['profile_canonical_sha256'],
                            budgets(capacity, 32 * GIB, 64 * MIB)['profile_canonical_sha256'])
        larger = plan([{'name': 'large', 'requirements': {'cpus': 1, 'memory_mib': 16384,
                       'disk_mib': 10240, 'build_timeout_sec': 600}}], 1, 32 * GIB, 64 * MIB)
        with self.assertRaises(ValueError):
            budgets(larger, 32 * GIB, 64 * MIB, profile=profile)

    def test_complete_profile_is_accepted_by_real_service_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            cfg = configured(Path(temporary).resolve(), budgets(inputs(), 32 * GIB, 64 * MIB))
        self.assertEqual(cfg['limits']['builder_disk_bytes'], 32 * GIB)
        self.assertEqual(cfg['limits']['runtime_cleanup_reserve_bytes'], 2 * GIB)
        self.assertEqual(cfg['runtime']['data_transfer'], json.loads(RUNTIME_PROFILE.read_text())['data_transfer'])
        self.assertGreater(cfg['builder']['data_transfer']['file_upload_bytes'], cfg['limits']['max_input_bytes'])

    def test_preflight_includes_both_pool_quotas_and_extra_live_disks(self):
        capacity = inputs(89)
        deployment = budgets(capacity, 32 * GIB, 64 * MIB)
        host = deployment['host']
        self.assertEqual(host['runtime_disk_bytes'], 40 * GIB)
        self.assertEqual(host['minimum_free_disk_bytes'], capacity['required_free_bytes'] + 40 * GIB)
        # A host that passed the old serial check cannot admit this four-VM profile.
        with self.assertRaisesRegex(ValueError, 'disk'):
            check_headroom(deployment, cpus=8, available_memory=32 * GIB,
                           free_disk=capacity['required_free_bytes'])
        exact = {'cpus': host['minimum_cpus'], 'available_memory': host['minimum_available_memory_bytes'],
                 'free_disk': host['minimum_free_disk_bytes']}
        check_headroom(deployment, **exact)
        for field in exact:
            with self.subTest(field=field), self.assertRaises(ValueError):
                check_headroom(deployment, **(exact | {field: exact[field] - 1}))
        self.assertFalse(host['control_allowance_enforced_by_cgroup'])

    def test_pending_jobs_reserve_disk_without_claiming_parallel_builders(self):
        p = load()
        p['builder']['max_pending'] = 2
        one = budgets(inputs(), 32 * GIB, 64 * MIB)
        two = budgets(inputs(), 32 * GIB, 64 * MIB, profile=p)
        self.assertEqual(two['builder']['max_active_global'], 1)
        self.assertEqual(two['host']['builder_vmm_memory_bytes'], one['host']['builder_vmm_memory_bytes'])
        self.assertGreater(two['host']['minimum_free_disk_bytes'], one['host']['minimum_free_disk_bytes'] + 50 * GIB)
        self.assertEqual(two['limits']['max_pending'], 2)

    def test_invalid_and_infeasible_profiles_are_refused(self):
        for section, field, value in (('runtime', 'max_active', True), ('builder', 'cpu_count', 0),
                ('host', 'memory_headroom_bytes', -1), ('runtime', 'max_vcpus', 3),
                ('runtime', 'max_host_memory_bytes', 8192 * MIB)):
            p = load()
            p[section][field] = value
            with self.subTest(section=section, field=field), self.assertRaises(ValueError):
                budgets(inputs(), 32 * GIB, 64 * MIB, profile=p)
        with self.assertRaisesRegex(ValueError, 'builder disk'):
            budgets(inputs(), 10 * GIB, 64 * MIB)
        p = load()
        p['runtime']['misspelled_budget'] = 1
        with self.assertRaises(ValueError):
            budgets(inputs(), 32 * GIB, 64 * MIB, profile=p)


class AcceptancePressureTests(unittest.TestCase):
    def test_full_http_budget_remains_queryable_and_releases_only_after_exact_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            cfg = configured(root, budgets(inputs(), 32 * GIB, 64 * MIB))
            limits = cfg['limits']
            backend = FakeBackend()
            backend.fail = True
            backend.cleanup_known = False
            store = BuildStore(root / 'builds', limits, clock=lambda: 1000, free_bytes=lambda: 1000 * GIB)
            server = BuildHTTP(('127.0.0.1', 0), store, [{'tenant': 1, 'secret': 'owner'}])
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            client = BuildClient('http://127.0.0.1:' + str(server.server_port), 'owner')
            request = {'kind': 'oci', 'image': 'ubuntu:24.04'}
            try:
                first = client.request('POST', '/sandproof/template-builds', request, token='original')
                job = first['buildID']
                Supervisor(store, backend).tick()
                path = '/sandproof/template-builds/' + job
                self.assertEqual(client.request('GET', path)['state'], 'CleanupUnknown')
                self.assertGreater(store.get(job)['reserved'], 0)
                self.assertEqual(client.request('GET', path + '/logs?offset=0&limit=1024')['text'], '')
                with self.assertRaisesRegex(RuntimeError, 'BuildBudgetExceeded'):
                    client.request('POST', '/sandproof/template-builds', request, token='next')
                replay = client.request('POST', '/sandproof/template-builds', request, token='original')
                self.assertEqual((replay['buildID'], replay['deadline']), (job, first['deadline']))
            finally:
                server.shutdown()
                server.server_close()
                thread.join()
                store.close()
            store = BuildStore(root / 'builds', limits, clock=lambda: 1001, free_bytes=lambda: 1000 * GIB)
            try:
                supervisor = Supervisor(store, backend)
                supervisor.tick()
                self.assertEqual(store.query(job, 1)['state'], 'CleanupUnknown')
                backend.cleanup_known = True
                supervisor.tick()
                self.assertEqual(store.query(job, 1)['state'], 'Failed')
                self.assertEqual(store.get(job)['reserved'], 0)
                self.assertEqual(backend.builds, [job])
                self.assertEqual(store.db.execute('SELECT COUNT(*) FROM jobs').fetchone()[0], 1)
                self.assertNotEqual(store.submit(1, request, 'next')['buildID'], job)
                self.assertEqual(store.query(job, 1)['deadline'], first['deadline'])
            finally:
                store.close()

    def test_interrupted_build_at_full_budget_does_not_run_user_steps_again(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            cfg = configured(root, budgets(inputs(), 32 * GIB, 64 * MIB))
            backend = FakeBackend()
            backend.power_loss = True
            store = BuildStore(root / 'builds', cfg['limits'], clock=lambda: 1000, free_bytes=lambda: 1000 * GIB)
            try:
                job = store.submit(1, {'kind': 'oci', 'image': 'ubuntu:24.04'}, 'original')['buildID']
                with self.assertRaises(PowerLoss):
                    Supervisor(store, backend).tick()
            finally:
                store.close()
            backend.power_loss = False
            store = BuildStore(root / 'builds', cfg['limits'], clock=lambda: 1001, free_bytes=lambda: 1000 * GIB)
            try:
                Supervisor(store, backend).tick()
                self.assertEqual(store.query(job, 1)['failure'], 'InterruptedBuild')
                self.assertEqual(backend.builds, [job])
                self.assertEqual(store.get(job)['reserved'], 0)
                self.assertFalse(backend.vms)
            finally:
                store.close()


if __name__ == '__main__':
    unittest.main()
