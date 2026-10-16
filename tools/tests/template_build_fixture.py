"""Explicit sole-mode build fixtures, using the actual compiled Rust reducer.

These budgets and helper identities are test inputs, never production defaults.
The production bridge still requires Linux; no alternate helper is introduced.
"""
from functools import lru_cache
import copy
import hashlib
import os
from pathlib import Path
import subprocess

from tools.template_build_store import BuildStore, MIB
from tools.template_build_client import BuildClient

ROOT = Path(__file__).resolve().parents[2]
PRIVATE_TEST_KEY = 'private-build-test-credential-000001'


class PrivateBuildClient(BuildClient):
    """Explicit protected helper caller in tests, not an alternative product CLI."""
    def headers(self):
        owner = {'owner': '1', 'synthetic': '1', 'other': '2'}.get(self.key, self.key)
        return {'x-sandproof-build-key': PRIVATE_TEST_KEY, 'x-sandproof-build-owner': owner}


def bounded_config(helper):
    return {'namespace': 17, 'windows': {'max_open_windows': 4, 'max_duration_ms': 600000},
            'retention_ms': 1000, 'max_request_bytes': 2048, 'max_result_bytes': 8192,
            'max_record_bytes': 65536, 'database_bytes': 8 * MIB, 'wal_bytes': 17 * MIB,
            'recovery_reserve_bytes': 8 * MIB, 'helper': dict(helper)}


def synthetic_helper():
    """Shape-only config fixture; must not be used to perform window actions."""
    return {'path': '/explicit-test-helper', 'sha256': 'a' * 64, 'owner_uid': os.geteuid(),
            'max_input_bytes': 65536, 'max_output_bytes': 65536, 'timeout_seconds': 5}


@lru_cache(maxsize=1)
def compiled_helper():
    subprocess.run(['cargo', '--config', 'scripts/check-profiles.toml',
                    'build', '--locked', '-p', 'sandproof-controller-service',
                    '--bin', 'sandproof-window-reducer'], cwd=ROOT, check=True, timeout=180)
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    if not target.is_absolute():
        target = ROOT / target
    binary = target / 'debug/sandproof-window-reducer'
    return synthetic_helper() | {'path': str(binary),
                                'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}


def new_store(root, limits, **kwargs):
    return BuildStore(root, limits, bounded=bounded_config(compiled_helper()), **kwargs)


def complete_pool_policy(root, runtime, builder, config):
    """Attach explicit physical/config identities to static synthetic pools.

    Test activation supplies the existing lease fixture, not host authorization.
    This function is not installed or called by product code.
    """
    from tools.tests.test_template_build_config import transfer_settings
    from tools.tests.test_template_physical_storage import activate_for_test
    kernel = Path(runtime['kernel'])
    registry = config['registry']
    registry.setdefault('max_templates_global', config['limits']['max_jobs'])
    config['bounded_builds'] = bounded_config(synthetic_helper())
    pools = []
    for index, (role, settings) in enumerate((('runtime', runtime), ('builder', builder))):
        settings.pop('max_vm_id', None)
        settings['namespace'] = f'{index + 1:016x}'
        settings['firecracker'] = str(kernel)
        settings['data_transfer'] = transfer_settings() | settings.get('data_transfer', {})
        settings['bounded_host'] = {'uid_base': settings['uid_base'], 'uplink_base': index * 16,
            'console_policy': 'discard', 'maintenance': {'max_retained_results': 16, 'max_host_slots': 2,
                'windows': {'max_open_windows': 4, 'max_duration_ms': 600000}}}
        settings['maintenance'] = {'capacity': {'max_online_storage_bytes': MIB}}
        for descriptor in settings['templates'].get('templates', []):
            descriptor.setdefault('kernel', {'size_bytes': kernel.stat().st_size})
        pools.append({'role': role, 'namespace': settings['namespace'],
            **{field: settings[field] for field in ('database', 'node_state_dir', 'chroot_base')},
            'rootfs_budget_bytes': settings['max_disk_bytes_global'] - settings['cleanup_reserve_bytes'],
            'max_host_slots': 2, 'kernel_copy_bytes': max(kernel.stat().st_size,
                registry['max_artifact_bytes_per_tenant'],
                *(item['kernel']['size_bytes'] for item in settings['templates'].get('templates', []))),
            'runtime_copy_bytes': kernel.stat().st_size, 'cleanup_reserve_bytes': settings['cleanup_reserve_bytes'],
            'host_metadata_bytes': MIB, 'host_metadata_inodes': 32, 'jail_inodes_per_slot': 12})
    inode_budget = 8 + 19 * config['limits']['max_jobs'] + 5 * registry['max_templates_global']
    profile = {'version': 1, 'coordination_file': str(root / 'physical-profile'), 'pools': pools,
        'public_build': {'workspace_owner_uid': os.geteuid(), 'registry_owner_uid': os.geteuid(), 'root': config['root'], 'registry_root': registry['root'],
            'workspace_bytes': config['limits']['workspace_bytes'],
            'registry_bytes': registry['max_artifact_bytes_global'] + registry['max_templates_global'] * (65536 + 87),
            'max_jobs': config['limits']['max_jobs'], 'max_templates': registry['max_templates_global'],
            'inode_budget': inode_budget},
        'host_template_caches': [],
        'service_data': [],
        'filesystem_overhead_bytes': (inode_budget + 512) * MIB,
        'filesystem_overhead_inodes': inode_budget + 512}
    for settings in (runtime, builder):
        settings['bounded_host']['physical_storage'] = copy.deepcopy(profile)
    Path(profile['coordination_file']).touch(mode=0o600)
    activate_for_test(profile)
    return profile
