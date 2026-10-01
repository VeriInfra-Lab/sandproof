"""Drop a real SDK create receipt, recover only its original KVM identity.

Prebuilt template component probe, not an OCI build or benchmark score. The
HTTPX wrapper calls the original transport, then drops one successful receipt.
Requires an isolated host and two exclusive delegated cgroup parents.
"""
import argparse
import asyncio
import contextlib
import httpx
import json
import os
from pathlib import Path
import secrets
import sqlite3
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import urllib.request

from harbor.models.task.config import NetworkMode
from benchmarks.generic_environment import GenericEnvironment
from tools import service_cgroup
from tools.runtime_create_recovery import matching_runtime, recover_runtime_create
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import fetch, free_loopback_port, start_server, stop_server, wait_ready
from server_template_isolation import audit_gone
from server_templates import descriptor, record_for
from server_vm_id_capacity import check_no_resources


def exercise(args):
    assert os.geteuid() == 0 and sys.platform == 'linux'
    checkout = Path(__file__).resolve().parents[2]
    git = ['git', '-C', str(checkout)]
    assert not subprocess.check_output(git + ['status', '--porcelain'])
    commit = subprocess.check_output(git + ['rev-parse', 'HEAD'], text=True).strip()
    service_cgroup.private(args.scratch_parent)
    assert args.uid_base >= 1000 and not processes_in_uid_range(args.uid_base, args.uid_base + 1)
    assert args.cgroup_parent != args.service_cgroup_parent
    for parent in (args.cgroup_parent, args.service_cgroup_parent):
        service_cgroup.filesystem(parent)
        assert not any(p.is_dir() for p in parent.iterdir())
    root = Path(tempfile.mkdtemp(prefix='cr-', dir=args.scratch_parent))
    print('evidence=' + str(root), flush=True)
    for name in ('db', 'state', 'jails', 'trial'):
        (root / name).mkdir(mode=0o700)
    namespace, build_id, key = secrets.token_hex(8), secrets.token_hex(16), secrets.token_hex(32)
    bound = descriptor('tpl-' + build_id, args.kernel, args.rootfs, 1, 256)
    view = {'templateId': bound['template_id'], 'revision': 'r1', 'cpuCount': 1, 'memoryMib': 256,
            'diskBytes': bound['resources']['disk_bytes'], 'hostMemoryBytes': 512 * 1024**2,
            'kernelSha256': bound['kernel']['sha256'], 'rootfsSha256': bound['rootfs']['sha256'],
            'startup': {'workdir': '/', 'user': 'root', 'env_vars': {}}}
    url = f'http://127.0.0.1:{free_loopback_port()}'
    settings = {'namespace': namespace, 'database': str(root / 'db/controller.db'),
        'node_state_dir': str(root / 'state'), 'chroot_base': str(root / 'jails'),
        'jailer': str(args.jailer), 'firecracker': str(args.firecracker),
        'kernel': str(args.kernel), 'rootfs': str(args.rootfs), 'uid_base': args.uid_base, 'max_vm_id': 1,
        'vcpu_count': 1, 'memory_mib': 256,
        'host_resources': {'cgroup_parent': str(args.cgroup_parent), 'memory_max_bytes': 512 * 1024**2},
        'file_policy_digest': 1, 'bind': url.removeprefix('http://'), 'domain': 'example.invalid',
        'credentials': [{'tenant': 1, 'secret': key}], 'max_active_global': 1, 'max_active_per_tenant': 1,
        'max_disk_bytes_global': args.rootfs.stat().st_size * 2,
        'max_disk_bytes_per_tenant': args.rootfs.stat().st_size * 2,
        'cleanup_reserve_bytes': 256 * 1024**2, 'max_timeout_secs': 86400, 'expiry_scan_seconds': 3600,
        'public_ipv4_enabled': True,
        'templates': {'templates': [bound], 'aliases': {'base': bound['template_id']}, 'compute_limits': {
            'max_vcpus_global': 1, 'max_vcpus_per_tenant': 1,
            'max_memory_bytes_global': 512 * 1024**2, 'max_memory_bytes_per_tenant': 512 * 1024**2}}}
    config = root / 'config.json'
    config.write_text(json.dumps(settings)); config.chmod(0o600)
    service_record = root / 'service-cgroup.json'
    service_cgroup.prepare(args.service_cgroup_parent, 'svc-' + namespace, service_record,
                           cpu_quota_us=100000, memory_bytes=2 * 1024**3)
    report = {'commit': commit, 'server_sha256': sha256_file(args.server), 'namespace': namespace,
              'uid_base': args.uid_base, 'template': view, 'scope': 'prebuilt KVM create receipt recovery',
              'drop_phase': args.drop_phase, 'status': 'failed', 'create_requests': 0}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    process = None
    try:
        process = start_server(args.server, config, root / 'server.log', None,
                               command_prefix=service_cgroup.prefix(service_record))
        wait_ready(opener, url, key, process)
        env = object.__new__(GenericEnvironment)
        env._network_policy = SimpleNamespace(network_mode=NetworkMode.PUBLIC)
        env.entry = {'requirements': {'build_timeout_sec': 120}}
        env.template, env.build_id, env.environment_name = view, build_id, 'receipt-recovery-probe'
        env.trial_paths = SimpleNamespace(trial_dir=root / 'trial')
        env.sdk_options = options(key, url, url)
        env._sandbox, env.sandbox_id, env.create_dispatched = None, None, False
        env._startup_env = lambda: {}
        async def prepared():
            pass  # Explicitly prebuilt: this probe does not exercise construction.
        env.prepare_template = prepared
        original_send = httpx.AsyncClient.send
        async def drop_receipt(client, request, *positional, **kwargs):
            assert str(request.url).startswith(url + '/')
            assert request.method == 'POST' and request.url.path == '/v2/sandboxes'
            report['create_requests'] += 1
            if args.drop_phase == 'starting':
                pending = asyncio.create_task(original_send(client, request, *positional, **kwargs))
                try:
                    deadline = time.monotonic() + 120
                    while time.monotonic() < deadline:
                        found = await asyncio.to_thread(matching_runtime, root / 'db/controller.db',
                            env.runtime_intent, owner=1, namespace=namespace)
                        if found is not None:
                            assert found['phase'] == 'Starting', 'required fault window was missed'
                            report['original_pending_identity'] = found
                            raise httpx.ReadError('injected lost pending create response', request=request)
                        await asyncio.sleep(.01)
                    raise TimeoutError('create admission was not observed')
                finally:
                    pending.cancel()
                    with contextlib.suppress(asyncio.CancelledError):
                        await pending
            response = await original_send(client, request, *positional, **kwargs)
            assert response.status_code == 201
            data = response.json()
            _, name, claim = record_for(root, SimpleNamespace(sandbox_id=data['sandboxID']))
            report['original_claim'] = claim
            report['resource_name'] = name
            report['service_live'] = service_cgroup.observe(service_record, pids=[process.pid])
            await response.aclose()
            raise httpx.ReadError('injected lost create response', request=request)
        httpx.AsyncClient.send = drop_receipt
        try:
            asyncio.run(env.start())
            raise AssertionError('create receipt unexpectedly reached adapter')
        except httpx.ReadError:
            report['lost_receipt_observed'] = True
        finally:
            httpx.AsyncClient.send = original_send
        assert report['create_requests'] == 1 and env.create_dispatched and env.sandbox_id is None
        recovery = recover_runtime_create(root / 'db/controller.db', env.runtime_intent,
            owner=1, namespace=namespace, deadline=time.monotonic() + 120,
            delete=lambda identity: fetch(opener, url, 'DELETE', '/sandboxes/' + identity, key)[0])
        report['recovery'] = recovery
        report['physical_audit'] = [audit_gone(root, args.firecracker, args.cgroup_parent, p.name)
                                    for p in (root / 'jails/.claims').iterdir()]
        check_no_resources(root, namespace, args.firecracker, args.uid_base)
        with sqlite3.connect(root / 'db/controller.db') as db:
            assert db.execute('SELECT COUNT(*) FROM sandboxes').fetchone()[0] == 1
            assert db.execute("SELECT COUNT(*) FROM outbox WHERE dispatch_state='dispatched'").fetchone()[0] == 0
        assert sha256_file(args.rootfs) == view['rootfsSha256']
        report['status'] = 'passed'
    finally:
        if process is not None:
            stop_server(process)
        if report['status'] == 'passed':
            report['service_cleanup'] = service_cgroup.remove(service_record)
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'evidence': str(root), 'commit': commit}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('server', 'jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent',
                 'cgroup-parent', 'service-cgroup-parent'):
        parser.add_argument('--' + name, type=lambda p: Path(p).resolve(strict=True), required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    parser.add_argument('--drop-phase', choices=('starting', 'running'), default='running')
    exercise(parser.parse_args())
