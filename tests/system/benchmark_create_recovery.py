"""Drop a real SDK create receipt, recover only its original KVM identity.

Prebuilt template component probe, not an OCI build or benchmark score. The
HTTPX wrapper calls the original transport, then drops one successful receipt.
Requires an isolated host and two exclusive delegated cgroup parents.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

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

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from harbor.models.task.config import NetworkMode
from benchmarks.generic_environment import GenericEnvironment
from tools import service_cgroup
from tools.runtime_create_recovery import find_runtime_create, recover_runtime_create
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_server_smoke import options
from server_process_crash import fetch, free_loopback_port, terminal_cleanup_confirmed
from _complete_runtime import complete_runtime
from _runtime_services import RuntimeServices, add_arguments
from _center_host_services import local_binding
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
        'kernel': str(args.kernel), 'rootfs': str(args.rootfs), 'uid_base': args.uid_base,
        'vcpu_count': 1, 'memory_mib': 256,
        'host_resources': {'cgroup_parent': str(args.cgroup_parent), 'memory_max_bytes': 512 * 1024**2},
        'file_policy_digest': 1, 'max_active_global': 1, 'max_active_per_tenant': 1,
        'max_disk_bytes_global': args.rootfs.stat().st_size * 2,
        'max_disk_bytes_per_tenant': args.rootfs.stat().st_size * 2,
        'cleanup_reserve_bytes': 256 * 1024**2, 'max_timeout_secs': 86400,
        'max_lifetime_secs': 86400, 'startup_timeout_secs': 300, 'expiry_scan_seconds': 1,
        'public_ipv4_enabled': True,
        'templates': {'templates': [bound], 'aliases': {'base': bound['template_id']}, 'compute_limits': {
            'max_vcpus_global': 1, 'max_vcpus_per_tenant': 1,
            'max_memory_bytes_global': 512 * 1024**2, 'max_memory_bytes_per_tenant': 512 * 1024**2}}}
    settings = complete_runtime(settings, slots=1)
    service_record = args.service_cgroup_record
    registry = dict(root=str(root/'unused-registry'), max_templates_global=1,
        max_templates_per_tenant=1, max_artifact_bytes_global=args.rootfs.stat().st_size*2,
        max_artifact_bytes_per_tenant=args.rootfs.stat().st_size*2,
        max_template_disk_bytes=args.rootfs.stat().st_size, validation_timeout_secs=120)
    services = RuntimeServices.prepare(args, root, [settings], [settings['templates']], key,
        url.removeprefix('http://'), service_record, job_count=2, registry_config=registry)
    report = {'commit': commit, 'center_sha256': sha256_file(args.center),
              'host_sha256': sha256_file(args.host), 'namespace': namespace,
              'uid_base': args.uid_base, 'template': view, 'scope': 'prebuilt KVM create receipt recovery',
              'drop_phase': args.drop_phase, 'status': 'failed', 'create_requests': 0}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    native_identity = None

    def observe_pending_native(intent):
        # Diagnostic fault-window observation only. No global identity, cleanup
        # or authorization is inferred from these local rows. Recovery below
        # uses the ORIGINAL checked center/host audits and their full binding.
        with contextlib.closing(sqlite3.connect(
                (root/'db/controller.db').as_uri()+'?mode=ro', uri=True)) as db:
            rows = db.execute('SELECT s.sandbox_id,s.state_blob,o.options_json FROM sandboxes s '
                              'JOIN sandbox_create_options o USING(sandbox_id)').fetchall()
        matches = [(identity,json.loads(state)) for identity,state,options_blob in rows
                   if json.loads(options_blob).get('metadata') == intent['metadata']]
        assert len(matches) <= 1, 'ambiguous original pending native observation'
        if matches:
            identity,state = matches[0]
            assert state['phase'] == 'Starting', 'required fault window was missed'
            return dict(native_id=identity, phase='Starting')
        return None

    def capture(identity, center, host):
        nonlocal native_identity
        row = [r for r in host['operations'] if r['current'] and
               r['identity']['sandbox_id'] == identity['sandbox_id']]
        assert len(row) == 1, 'original host responsibility unknown'
        if row[0]['local'] is None:
            report['no_local_admission'] = True
            return
        local = local_binding(host, identity['sandbox_id'], expected_node=identity['node'])
        native = [value for value in host['native'] if value['sandbox'] == local['sandbox']]
        assert len(native) == 1 and native[0]['owner'] == local['owner']
        if native[0]['phase'] == 'Starting':
            return False  # Observe the original preparation; never issue Create again.
        native_identity = f"sb-{namespace}-{local['sandbox']}"
        services.processes.stop('host-0')
        try:
            assert services.native_id_for(identity['sandbox_id']) == native_identity
            secret, name, claim = record_for(root, native_id=native_identity)
            actual = secret['template']
            assert actual['template_id'] == view['templateId'] and actual['revision'] == view['revision']
            assert actual['kernel']['sha256'] == view['kernelSha256']
            assert actual['rootfs']['sha256'] == view['rootfsSha256']
            assert actual['resources'] == dict(cpu_count=view['cpuCount'],memory_mib=view['memoryMib'],
                disk_bytes=view['diskBytes'],host_memory_bytes=view['hostMemoryBytes'])
            claim['network_inode'] = json.loads((root/'jails/.claims'/name/'network.json').read_text())['namespace_inode']
        finally:
            services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        report['cleanup_identity'] = claim
        report.setdefault('delete_observations', []).append(dict(identity=identity, local=local))
    try:
        services.start()
        services.wait_ready()
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
                        found = await asyncio.to_thread(observe_pending_native, env.runtime_intent)
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
            report['service_live'] = services.processes.observe()
            report['original_create_identity'] = find_runtime_create(services.planned_audit('center'),
                env.runtime_intent['metadata'], deployment=services.center_config['deployment_id'])
            assert report['original_create_identity']['sandbox_id'] == data['sandboxID']
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
        def read_host(node):
            assert node == services.host_configs[0]['inbox']['node']
            return services.planned_audit('host-0')
        recovery = recover_runtime_create(env.runtime_intent,
            read_center=lambda: services.planned_audit('center'), read_host=read_host,
            owner=1, deployment=services.center_config['deployment_id'],
            deadline=time.monotonic()+120, before_delete=capture,
            create_identity=report.get('original_create_identity'),
            delete=lambda identity: fetch(opener, url, 'DELETE', '/sandboxes/'+identity, key)[0])
        report['recovery'] = recovery
        assert recovery['status'] == 'destroyed' and recovery['delete_acknowledged']
        assert len(report.get('delete_observations', [])) + int(report.get('no_local_admission',False)) == 1
        if args.drop_phase == 'starting':
            assert report['original_pending_identity']['phase'] == 'Starting'
        original = report.get('cleanup_identity')
        report['physical_audit'] = ([audit_gone(root,args.firecracker,args.cgroup_parent,
            original['resource_id'],original)] if original is not None else [])
        check_no_resources(root,namespace,args.firecracker,args.uid_base,
            [original] if original is not None else [],native=services.native_pools[0])
        if native_identity is not None:
            assert terminal_cleanup_confirmed(root/'db/controller.db',native_identity)
        else:
            assert not report['physical_audit'] and report['no_local_admission']
        assert sha256_file(args.rootfs) == view['rootfsSha256']
        report['status'] = 'passed'
    finally:
        services.processes.stop_all()
        if report['status'] == 'passed':
            report['service_cleanup'] = services.cleanup_staging()
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'evidence': str(root), 'commit': commit}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent',
                 'cgroup-parent', 'service-cgroup-parent'):
        parser.add_argument('--' + name, type=lambda p: Path(p).resolve(strict=True), required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    parser.add_argument('--drop-phase', choices=('starting', 'running'), default='running')
    add_arguments(parser)
    exercise(parser.parse_args())
