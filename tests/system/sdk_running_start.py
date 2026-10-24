"""One real KVM VM: Running publication origin and same-VM restart lifetime.

Run as root in an operator-selected isolated short directory. On failure this
script preserves only its own test root for exact-resource inspection.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
from tools.capture_acceptance import original_chroot
import secrets
import shutil
import sys
import tempfile

from e2b import Sandbox

from sdk_server_smoke import options
from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _runtime_services import RuntimeServices, add_arguments
from server_process_crash import free_loopback_port, process_start_ticks
from server_templates import descriptor


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main(args):
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise RuntimeError('requires root on an isolated Linux/KVM host')
    assert importlib.metadata.version('e2b') == '2.51.0'
    parent = args.scratch_parent.resolve(strict=True)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 3 + 256 * 1024 * 1024:
        raise RuntimeError('insufficient room for private VM disk and cleanup')
    root = Path(tempfile.mkdtemp(prefix='sp9-', dir=parent))
    namespace = secrets.token_hex(8)
    vm_name = f'vm-{namespace}-0'
    firecracker_name = args.firecracker.resolve(strict=True).name
    socket = root / 'p/v' / firecracker_name / f'vm-{namespace}-{2**64 - 1}' / 'root/api.socket'
    if len(os.fsencode(socket)) >= 108:
        root.rmdir()
        raise RuntimeError('private Firecracker socket path is too long')
    key = secrets.token_hex(32)
    port = free_loopback_port()
    for name in ('db', 'state', 'p'):
        (root / name).mkdir(mode=0o700)
    kernel = root / 'vmlinux'
    rootfs = root / 'rootfs.ext4'
    for source, target in ((args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source, target)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    base_digest = digest(rootfs)
    template = descriptor('runtime-base', kernel, rootfs, 1, 256)
    catalog = {'templates': [template], 'aliases': {'base': template['template_id']},
               'compute_limits': {'max_vcpus_global': 2, 'max_vcpus_per_tenant': 1,
                                  'max_memory_bytes_global': 1024 * 1024**2,
                                  'max_memory_bytes_per_tenant': 512 * 1024**2}}
    settings = {
        'namespace': namespace,
        'database': str(root / 'db/controller.db'),
        'node_state_dir': str(root / 'state'),
        'jailer': str(args.jailer.resolve(strict=True)),
        'firecracker': str(args.firecracker.resolve(strict=True)),
        'chroot_base': str(root / 'p/v'),
        'kernel': str(kernel),
        'rootfs': str(rootfs),
        'uid_base': args.uid_base,
        'vcpu_count': 1,
        'memory_mib': 256,
        'file_policy_digest': 1,
        'max_active_global': 2,
        'max_active_per_tenant': 1,
        'max_disk_bytes_global': disk_bytes * 3,
        'max_disk_bytes_per_tenant': disk_bytes,
        'cleanup_reserve_bytes': disk_bytes,
        'max_timeout_secs': 600,
        'max_lifetime_secs': 600,
        'startup_timeout_secs': 300,
        'public_ipv4_enabled': False,
        'templates': catalog,
        'default_timeout_secs': 300,
        'expiry_scan_seconds': 1,
    }
    settings = complete_runtime(settings, slots=2)
    base_url = f'http://127.0.0.1:{port}'
    registry = {'root': str(root / 'registry'), 'max_templates_global': 1,
        'max_templates_per_tenant': 1, 'max_artifact_bytes_global': disk_bytes + kernel.stat().st_size,
        'max_artifact_bytes_per_tenant': disk_bytes + kernel.stat().st_size, 'validation_timeout_secs': 60}
    services = RuntimeServices.prepare(args, root=root, native_pools=[settings],
        registered_catalogs=[catalog], key=key, public_address=f'127.0.0.1:{port}',
        record=args.service_cgroup_record, job_count=4, private_builds=None, registry_config=registry)
    completed = False
    try:
        services.start()
        services.wait_ready()
        sdk = options(key, base_url, base_url)
        before_request = datetime.now(timezone.utc)
        sandbox = Sandbox.create(template='base', timeout=60,
                                 allow_internet_access=False, **sdk)
        after_response = datetime.now(timezone.utc)
        initial = sandbox.get_info()
        assert before_request <= initial.started_at <= after_response
        assert initial.end_at - initial.started_at == timedelta(seconds=60)
        record = json.loads((original_chroot(root) / '.claims' / vm_name / 'record.json').read_text())
        assert record['resource_id'] == vm_name
        assert process_start_ticks(record['pid']) == record['start_time_ticks']
        services.processes.stop('center')
        assert process_start_ticks(record['pid']) == record['start_time_ticks']
        services.processes.start('center')
        services.wait_ready(for_admission=False)
        resumed = Sandbox.connect(sandbox.sandbox_id, **sdk)
        renewed = resumed.get_info()
        assert renewed.started_at == initial.started_at
        assert renewed.end_at > initial.end_at
        services.processes.stop('host-0')
        assert process_start_ticks(record['pid']) == record['start_time_ticks']
        native_id = services.native_id_for(sandbox.sandbox_id)
        assert native_id == f'sb-{namespace}-1'
        services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        resumed = Sandbox.connect(sandbox.sandbox_id, **sdk)
        assert resumed.get_info().started_at == initial.started_at
        resumed.set_timeout(1200)
        assert resumed.get_info().end_at == initial.started_at + timedelta(seconds=600)
        _kill_confirmed = (Sandbox.kill(sandbox.sandbox_id, **sdk))
        assert _kill_confirmed
        assert terminal_cleanup_confirmed(root / 'db/controller.db', native_id)
        assert process_start_ticks(record['pid']) != record['start_time_ticks']
        assert not Path('/run/netns', vm_name).exists()
        assert not (original_chroot(root) / firecracker_name / vm_name).exists()
        assert digest(rootfs) == base_digest
        completed = True
        print(json.dumps({'center_sha256': digest(args.center), 'host_sha256': digest(args.host),
                          'base_sha256': base_digest,
                          'sdk_version': '2.51.0', 'durable_destroyed': 1,
                          'same_vm_center_restart': True, 'same_vm_host_restart': True,
                          'original_process_stopped': True,
                          'private_netns_jail': 'absent'}))
    finally:
        services.stop_all()
        if completed:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f'Exact failed-run artifacts retained: {root}', file=sys.stderr)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for name in ('jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent'):
        parser.add_argument(f'--{name}', required=True, type=Path)
    parser.add_argument('--uid-base', required=True, type=int)
    main(parser.parse_args())
