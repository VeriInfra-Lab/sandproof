"""Isolated KVM acceptance of original tasks via self-service templates.

Uses two independent controller pools and an exclusive namespace/UID range.
Retains original failures and audit evidence. No manual task recipes/models.
"""
import argparse
import asyncio
import contextlib
import http.client
import json
import os
import re
from pathlib import Path
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
from types import SimpleNamespace

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from server_process_crash import fetch, free_loopback_port, process_start_ticks, start_server, stop_server, wait_ready
from server_templates import descriptor, record_for
from server_template_isolation import audit_gone
from server_vm_id_capacity import check_no_resources
from template_self_service import BUILD_TOOLS
from tools.benchmark_tasks import verify
from tools.generic_capacity import plan
from tools.template_build_config import UPLOAD_ENVELOPE_BYTES
from benchmarks.artifact_budget import add_artifact_arguments, artifact_limits_from_args
from benchmarks.check_harbor import check_runtime, check_tasks
from benchmarks.run_generic import run_selected

MIB = 1024**2


def exercise(args):
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise RuntimeError('exclusive root Linux/KVM required')
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain']).strip():
        raise RuntimeError('clean exact source commit required')
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    lock = verify(json.loads((checkout / 'benchmarks/terminal-bench-2.0.lock.json').read_text()), args.registry, args.source)
    runtimes = check_runtime()
    check_tasks(lock, args.source)
    selected = {t['name'] for t in lock['tasks']} if args.all else set(args.task)
    if not selected or selected - {t['name'] for t in lock['tasks']}:
        raise RuntimeError('unknown original task')
    capacity = plan([t for t in lock['tasks'] if t['name'] in selected], args.repetitions,
                    args.rootfs.stat().st_size, args.kernel.stat().st_size,
                    artifact_bytes=args.artifact_limits['max_bytes'])
    # Trusted operator-provided ext4 base; read metadata without mounting it.
    superblock = subprocess.check_output(['/usr/sbin/dumpe2fs', '-h', str(args.rootfs)],
                                        text=True, env={**os.environ, 'LC_ALL': 'C'}, stderr=subprocess.DEVNULL)
    fields = dict(line.split(':', 1) for line in superblock.splitlines() if ':' in line)
    if int(fields['Free blocks']) * int(fields['Block size']) < capacity['worker_required_free_bytes']:
        raise RuntimeError('builder base lacks free space for bounded import and candidate')
    stride = capacity['uid_stride']
    if args.uid_base < 1000 or args.uid_base + 2 * stride >= 2**32 - 1:
        raise RuntimeError('invalid exclusive UID ranges')
    if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
        raise RuntimeError('exclusive UID pools occupied')
    if Path('/proc/sys/net/ipv4/ip_forward').read_text().strip() != '1':
        raise RuntimeError('approved public forwarding required')
    for parent in (args.cgroup_parent, args.build_cgroup_parent, args.scratch_parent):
        if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
            raise RuntimeError('private root parents required')
    for parent in (args.cgroup_parent, args.build_cgroup_parent):
        if (parent / 'cgroup.procs').read_text().strip() or any(p.is_dir() for p in parent.iterdir()):
            raise RuntimeError('exclusive empty delegated cgroups required')
    if shutil.disk_usage(args.scratch_parent).free < capacity['required_free_bytes']:
        raise RuntimeError('selected task artifacts, workspace and cleanup reserve exceed free disk')
    available = int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines()
                         if line.startswith('MemAvailable:'))) * 1024
    if (available < (capacity['max_memory_mib'] + 8448 + 2048) * MIB
            or os.cpu_count() < capacity['max_cpu'] + 2):
        raise RuntimeError('host headroom insufficient for build and task pools')
    for name, digest in BUILD_TOOLS.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError('fixed tool mismatch: ' + name)
    root = Path(tempfile.mkdtemp(prefix='hg-', dir=args.scratch_parent))
    print(f'root={root} commit={commit}', flush=True)
    binary = root / 'server'
    shutil.copyfile(args.server, binary)
    binary.chmod(0o700)
    original = [sha256_file(p) for p in (args.kernel, args.rootfs)]
    namespaces = [args.namespace or secrets.token_hex(8), args.build_namespace or secrets.token_hex(8)]
    if (any(not re.fullmatch('[0-9a-f]{16}', n) for n in namespaces)
            or namespaces[0][:6] == namespaces[1][:6]):
        raise ValueError('disjoint deployment namespaces required')
    if any(len(os.fsencode(root / str(i) / 'jails' / args.firecracker.name /
                           f'vm-{n}-{stride - 1}' / 'root/api.socket')) >= 108
           for i, n in enumerate(namespaces)):
        raise RuntimeError('test path too long for Firecracker socket')
    (root / 'capacity.json').write_text(json.dumps(capacity, indent=2))
    ports = [free_loopback_port() for _ in range(3)]
    urls = [f'http://127.0.0.1:{p}' for p in ports]
    key, builder_key = secrets.token_hex(32), secrets.token_hex(32)
    firecracker = args.firecracker.resolve(strict=True)
    settings = []
    for i in range(2):
        pool = root / str(i)
        pool.mkdir(mode=0o700)
        for name in ('db', 'state', 'jails'):
            (pool / name).mkdir(mode=0o700)
        cpu = 2 if i else 1
        memory = 8192 if i else 2048
        base = descriptor('builder-base' if i else 'runtime-base', args.kernel, args.rootfs, cpu, memory)
        cfg = {'namespace': namespaces[i], 'database': str(pool / 'db/controller.db'),
            'node_state_dir': str(pool / 'state'), 'chroot_base': str(pool / 'jails'),
            'jailer': str(args.jailer), 'firecracker': str(firecracker),
            'kernel': str(args.kernel), 'rootfs': str(args.rootfs), 'uid_base': args.uid_base + stride * i,
            'max_vm_id': stride - 1, 'vcpu_count': cpu, 'memory_mib': memory,
            'host_resources': {'cgroup_parent': str(args.build_cgroup_parent if i else args.cgroup_parent), 'memory_max_bytes': (memory + 256) * MIB},
            'file_policy_digest': 1, 'bind': f'127.0.0.1:{ports[i]}', 'domain': 'example.invalid',
            'credentials': [{'tenant': 1, 'secret': builder_key if i else key}],
            'max_active_global': 2, 'max_active_per_tenant': 2,
            'max_disk_bytes_global': 2 * args.rootfs.stat().st_size, 'max_disk_bytes_per_tenant': 2 * args.rootfs.stat().st_size,
            'cleanup_reserve_bytes': capacity['cleanup_bytes'], 'max_timeout_secs': capacity['max_build_seconds'] + 60 if i else 86400,
            'max_lifetime_secs': capacity['max_build_seconds'] + 60 if i else 86400,
            'startup_timeout_secs': capacity['max_build_seconds'],
            'expiry_scan_seconds': 2, 'public_ipv4_enabled': True,
            'templates': {'templates': [base], 'aliases': {'base': base['template_id']},
                'compute_limits': {'max_vcpus_global': 8, 'max_vcpus_per_tenant': 8,
                    'max_memory_bytes_global': 18 * 1024 * MIB, 'max_memory_bytes_per_tenant': 18 * 1024 * MIB}},
            'data_transfer': {'file_upload_bytes': capacity['input_bytes'] + UPLOAD_ENVELOPE_BYTES if i else 512 * MIB,
                'file_download_bytes': (16 * 1024 * MIB if i else 96 * MIB),
                'command_input_bytes': MIB, 'command_output_bytes': 32 * MIB,
                'rpc_request_bytes': MIB, 'rpc_response_bytes': MIB, 'max_transfers': 16,
                'max_transfers_per_vm': 8, 'control_reserve': 1,
                'request_idle_seconds': 13000, 'file_idle_seconds': 60, 'write_idle_seconds': 13000}}
        if i:
            cfg['internal_build_vms'] = True
        else:
            for p in (root / 'registry', root / 'registry/inbox', root / 'registry/artifacts'):
                p.mkdir(mode=0o700)
            cfg['template_registry'] = {'root': str(root / 'registry'), 'max_templates_global': capacity['jobs'],
                'max_templates_per_tenant': capacity['jobs'], 'max_artifact_bytes_global': capacity['registry_bytes'],
                'max_artifact_bytes_per_tenant': capacity['registry_bytes'], 'validation_timeout_secs': 45}
        settings.append(cfg)
        (pool / 'config.json').write_text(json.dumps(cfg))
        (pool / 'config.json').chmod(0o600)
    (root / 'builds').mkdir(mode=0o700)
    build = {'root': str(root / 'builds'), 'bind': f'127.0.0.1:{ports[2]}',
        'runtime_config': str(root / '0/config.json'), 'builder_config': str(root / '1/config.json'),
        'tools': {n: {'path': str(args.tools / n), 'sha256': d} for n, d in BUILD_TOOLS.items()},
        'dns_servers': ['1.1.1.1', '8.8.8.8'],
        'limits': {'max_cpu': capacity['max_cpu'], 'max_memory_mib': capacity['max_memory_mib'],
            'max_disk_mib': capacity['max_disk_mib'], 'timeout_seconds': capacity['max_build_seconds'],
            'max_jobs': capacity['jobs'], 'max_jobs_per_tenant': capacity['jobs'],
            'max_pending': 1, 'max_pending_per_tenant': 1,
            'workspace_bytes': capacity['workspace_bytes'], 'workspace_bytes_per_tenant': capacity['workspace_bytes'],
            'max_input_bytes': capacity['input_bytes'], 'max_expanded_bytes': capacity['expanded_bytes'],
            'max_entries': 200000, 'max_log_bytes': capacity['log_bytes']}}
    (root / 'build.json').write_text(json.dumps(build))
    (root / 'build.json').chmod(0o600)
    controllers, supervisor, log = [], None, None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    rows, complete, all_passed = [], False, True
    def audit_shape(env):
        template = env.template
        secret, name, claim = record_for(root / '0', env._sandbox)
        bound = secret['template']
        expected = {'cpu_count': template['cpuCount'], 'memory_mib': template['memoryMib'],
                    'disk_bytes': template['diskBytes'], 'host_memory_bytes': template['hostMemoryBytes']}
        assert bound['resources'] == expected
        assert bound['template_id'] == template['templateId'] and bound['revision'] == template['revision']
        assert bound['rootfs']['sha256'] == template['rootfsSha256']
        assert process_start_ticks(claim['pid']) == claim['start_time_ticks']
        disk = root / '0/jails' / firecracker.name / name / 'root/rootfs.ext4'
        assert disk.stat().st_size == template['diskBytes']
        source = Path(bound['rootfs']['path'])
        assert (disk.stat().st_dev, disk.stat().st_ino) != (source.stat().st_dev, source.stat().st_ino)
        cgroup = args.cgroup_parent / name
        assert int((cgroup / 'memory.max').read_text()) == template['hostMemoryBytes']
        assert (cgroup / 'cpu.max').read_text().strip() == f"{template['cpuCount'] * 100000} 100000"
        assert str(claim['pid']) in (cgroup / 'cgroup.procs').read_text().split()
        code, data = fetch(opener, urls[0], 'GET', '/sandboxes/' + env.sandbox_id, key)
        assert code == 200 and json.loads(data)['diskSizeMB'] * MIB == template['diskBytes']
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(5)
            connection.connect(str(disk.parent / 'api.socket'))
            connection.sendall(b'GET /machine-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
            with http.client.HTTPResponse(connection) as response:
                response.begin()
                assert response.status == 200
                machine = json.loads(response.read(65536))
        assert (machine['vcpu_count'], machine['mem_size_mib']) == (template['cpuCount'], template['memoryMib'])
        return {'template_id': template['templateId'], 'revision': template['revision'], 'resources': expected,
                'pid': claim['pid'], 'start_time_ticks': claim['start_time_ticks'],
                'disk_inode': disk.stat().st_ino, 'machine': machine}
    def completion_audit(row):
        # A lost reply may precede completion of the same admitted disk copy.
        # Observe through its original deadline plus a bounded cleanup margin.
        original_deadline = (row.get('build') or {}).get('deadline', time.time())
        # Publication can admit its own check VM after a slow artifact check.
        # Keep both controllers alive through that VM's bounded preparation
        # and lifetime, rather than killing the reclaimer after a 90s pause.
        deadline = (time.monotonic() + max(0, original_deadline - time.time())
                    + 2 * capacity['max_build_seconds'] + 90)
        while True:
            with sqlite3.connect(root / 'builds/builds.db') as db:
                pending = db.execute('SELECT COUNT(*) FROM jobs WHERE reserved>0').fetchone()[0]
            if not pending:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('original build still owns its reservation')
            time.sleep(0.5)
        for i in range(2):
            pool = root / str(i)
            with sqlite3.connect(pool / 'db/controller.db') as db:
                phases = [json.loads(r[0])['phase'] for r in db.execute('SELECT state_blob FROM sandboxes')]
                pending_actions = db.execute("SELECT COUNT(*) FROM outbox WHERE dispatch_state='dispatched'").fetchone()[0]
            if pending_actions or any(p != 'Destroyed' for p in phases):
                raise RuntimeError('pool still owns an unreclaimed identity')
            claims = pool / 'jails/.claims'
            if claims.exists():
                check_no_resources(pool, namespaces[i], firecracker, args.uid_base + i * stride)
                for claim in claims.iterdir():
                    audit_gone(pool, firecracker, args.build_cgroup_parent if i else args.cgroup_parent, claim.name)
        if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
            raise RuntimeError('assigned UID still owns a live process')
        result = {'exact_cleanup': 'passed'}
        if row.get('build_dispatched'):
            from tools.template_build_client import BuildClient
            client = BuildClient(urls[2], key)
            # Reobserve the original idempotent request. A POST is forbidden.
            if row.get('build_id'):
                result['build'] = client.request('GET', '/sandproof/template-builds/' + row['build_id'])
            else:
                raise RuntimeError('unknown build submission requires identity reconciliation')
            if result['build']['state'] not in ('Ready', 'Failed'):
                raise RuntimeError('build state remains uncertain after physical audit')
        template = row.get('template') or (result.get('build') or {}).get('template')
        if template:
            source = root / 'registry/artifacts' / template['templateId'].removeprefix('tpl-') / 'rootfs.ext4'
            assert sha256_file(source) == template['rootfsSha256']
        return result

    try:
        for i in range(2):
            p = start_server(binary, root / str(i) / 'config.json', root / str(i) / 'server.log', None)
            controllers.append(p)
            wait_ready(opener, urls[i], builder_key if i else key, p)
        log = (root / 'build-service.log').open('ab')
        supervisor = subprocess.Popen([sys.executable, str(checkout / 'tools/template_build_service.py'),
            '--config', str(root / 'build.json')], stdout=log, stderr=subprocess.STDOUT)
        for _ in range(60):
            try:
                if fetch(opener, urls[2], 'GET', '/missing', key)[0] == 404:
                    break
            except OSError:
                if supervisor.poll() is not None:
                    raise RuntimeError('build supervisor startup failed')
                time.sleep(0.5)
        else:
            raise RuntimeError('build endpoint did not become ready')
        for repetition in range(args.repetitions):
            output = root / f'run-{repetition}'
            output.mkdir(mode=0o700)
            (output / 'inputs.json').write_text(json.dumps({'source': lock['source'], 'runtimes': runtimes,
                'source_kind': args.source_kind, 'artifact_limits': args.artifact_limits,
                'task_files': {t['name']: t['files_sha256'] for t in lock['tasks']}}, indent=2))
            run_args = SimpleNamespace(source=args.source, output=output, task=sorted(selected), all=args.all,
                api_url=urls[0], build_url=urls[2], source_kind=args.source_kind,
                artifact_limits=args.artifact_limits, start_audit=audit_shape, completion_audit=completion_audit)
            old_key = os.environ.get('SANDPROOF_TEST_API_KEY')
            os.environ['SANDPROOF_TEST_API_KEY'] = key
            with (root / f'run-{repetition}.log').open('wb') as out:
                import io
                text_out = io.TextIOWrapper(out, write_through=True)
                try:
                    with contextlib.redirect_stdout(text_out), contextlib.redirect_stderr(text_out):
                        asyncio.run(run_selected(run_args, lock))
                finally:
                    text_out.detach()
                    if old_key is None:
                        os.environ.pop('SANDPROOF_TEST_API_KEY', None)
                    else:
                        os.environ['SANDPROOF_TEST_API_KEY'] = old_key
            report = json.loads((output / 'full-set.json').read_text())
            current = [r for r in report['tasks'] if r['status'] != 'not_run']
            # Exact reclaim, not just the SDK's kill boolean.
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline:
                with sqlite3.connect(root / 'builds/builds.db') as db:
                    pending = db.execute('SELECT COUNT(*) FROM jobs WHERE reserved>0').fetchone()[0]
                if pending == 0:
                    break
                time.sleep(0.5)
            if pending:
                raise RuntimeError('original build identity still owns cleanup budget')
            for i in range(2):
                with sqlite3.connect(root / str(i) / 'db/controller.db') as db:
                    phases = [json.loads(r[0])['phase'] for r in db.execute('SELECT state_blob FROM sandboxes')]
                if any(p != 'Destroyed' for p in phases):
                    raise RuntimeError('durable terminal cleanup incomplete')
                claims = root / str(i) / 'jails/.claims'
                if claims.exists():
                    check_no_resources(root / str(i), namespaces[i], firecracker, args.uid_base + i * stride)
                    for claim in claims.iterdir():
                        audit_gone(root / str(i), firecracker, args.build_cgroup_parent if i else args.cgroup_parent, claim.name)
                else:
                    for parent in (root / str(i) / 'jails', Path('/run/netns'), Path('/sys/fs/cgroup')):
                        assert not list(parent.rglob(f'vm-{namespaces[i]}-*'))
            if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
                raise RuntimeError('exact pool UID process remains')
            for row in current:
                row['repetition'] = repetition
                template = row.get('template') or (row.get('build') or {}).get('template')
                if template:
                    source = root / 'registry/artifacts' / template['templateId'].removeprefix('tpl-') / 'rootfs.ext4'
                    assert sha256_file(source) == template['rootfsSha256']
            rows.extend(current)
            report['exact_cleanup_audit'] = ('passed' if all(r.get('exact_cleanup') == 'passed' for r in current)
                                             else 'unknown')
            (output / 'full-set.json').write_text(json.dumps(report, indent=2) + '\n')
            (root / 'report.json').write_text(json.dumps({'commit': commit, 'server_sha256': sha256_file(binary),
                'kernel_sha256': original[0], 'builder_base_sha256': original[1], 'tools': BUILD_TOOLS,
                'source_kind': args.source_kind, 'tasks': rows, 'fixed_task_count': 89,
                'no_process_network_cgroup_jail_residue': True,
                'original_oracle_verifier': True, 'paid_model_used': False}, indent=2) + '\n')
            passed = len(current) == len(selected) and all(r['status'] == 'passed' for r in current)
            all_passed = all_passed and passed
            print(json.dumps({'repetition': repetition, 'execution_passed': passed,
                              'tasks': [{k: r.get(k) for k in ('name', 'status', 'stage', 'error_type', 'exact_cleanup')} for r in current]}), flush=True)
            if not passed:
                print('original task failures retained; no pass claim for this batch', flush=True)
            if any(r.get('exact_cleanup') != 'passed' for r in current):
                raise RuntimeError('unresolved per-task identity; no further repetition permitted')
        assert original == [sha256_file(p) for p in (args.kernel, args.rootfs)]
        complete = True
        print('generic task run and cleanup complete; report=' + str(root / 'report.json'), flush=True)
        return 0 if all_passed else 1
    finally:
        if supervisor:
            stop_server(supervisor)
        if log:
            log.close()
        for p in reversed(controllers):
            stop_server(p)
        # Preserve private credentials until the operator independently audits
        # both pools, including failures; deleting them can prevent recovery.
        if not complete:
            print('failure evidence retained at ' + str(root), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('server', 'firecracker', 'jailer', 'kernel', 'rootfs', 'tools', 'scratch-parent',
                 'cgroup-parent', 'build-cgroup-parent', 'registry', 'source'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument('--task', action='append')
    selection.add_argument('--all', action='store_true')
    parser.add_argument('--namespace')
    parser.add_argument('--build-namespace')
    parser.add_argument('--source-kind', choices=('oci', 'dockerfile'), default='oci')
    parser.add_argument('--repetitions', type=int, choices=(1, 2), default=1)
    parser.add_argument('--temporarily-forward', action='store_true')
    add_artifact_arguments(parser)
    args = parser.parse_args()
    args.artifact_limits = artifact_limits_from_args(args)
    forward = Path('/proc/sys/net/ipv4/ip_forward').read_text().strip()
    try:
        if forward == '0' and args.temporarily_forward:
            subprocess.run(['/usr/sbin/sysctl', '-w', 'net.ipv4.ip_forward=1'], check=True)
        sys.exit(exercise(args))
    finally:
        if forward == '0' and args.temporarily_forward:
            subprocess.run(['/usr/sbin/sysctl', '-w', 'net.ipv4.ip_forward=0'], check=True)
