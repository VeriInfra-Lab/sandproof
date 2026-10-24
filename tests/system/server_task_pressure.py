"""Three-VM/two-tenant task-budget pressure and host observations on real KVM.

Run as root with approved inputs, an empty delegated cgroup and exclusive UIDs.
Reports service/relay RSS separately from VMM cgroup charges and host headroom.
These observations are not a production memory cap or a final deployment budget.
Failure retains exact artifacts; only known SDK identities are killed.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()


import argparse
import asyncio
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
from tools.capture_acceptance import original_chroot
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

from e2b import AsyncSandbox, Sandbox

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from sdk_command_input import call
from sdk_server_smoke import options
from sdk_task_global_pressure import exercise as pressure
from _complete_runtime import complete_runtime, terminal_cleanup_confirmed
from _runtime_services import RuntimeServices, add_arguments
from server_process_crash import free_loopback_port, process_start_ticks
from server_templates import descriptor, record_for


MIB = 1024 * 1024
VM_COUNT = 6


def host_headroom(root):
    available = next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines()
                     if line.startswith('MemAvailable:'))
    return {'memory_available_bytes': int(available) * 1024,
            'disk_available_bytes': shutil.disk_usage(root).free}


def network_state():
    def stable(value):
        if isinstance(value, dict):
            return {key: stable(item) for key, item in value.items()
                    if key not in ('metainfo', 'packets', 'bytes')}
        if isinstance(value, list):
            return [stable(item) for item in value
                    if not isinstance(item, dict) or 'metainfo' not in item]
        return value
    rules = json.loads(subprocess.check_output(['/usr/sbin/nft', '-j', 'list', 'ruleset']))
    return {'rules': stable(rules),
            'interfaces': sorted(path.name for path in Path('/sys/class/net').iterdir()),
            'forward': Path('/proc/sys/net/ipv4/ip_forward').read_text()}


def process_tree(server_pid, server_ticks, vm_pids):
    assert process_start_ticks(server_pid) == server_ticks
    rows = {}
    for proc in Path('/proc').iterdir():
        if not proc.name.isdecimal():
            continue
        try:
            fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
            if fields[0] != 'Z':
                rows[int(proc.name)] = {'pid': int(proc.name), 'parent': int(fields[1]),
                                       'start_ticks': int(fields[19]),
                                       'rss_bytes': int(fields[21]) * os.sysconf('SC_PAGE_SIZE')}
        except (FileNotFoundError, ProcessLookupError):
            pass
    descendants = {server_pid}
    while True:
        expanded = descendants | {pid for pid, row in rows.items() if row['parent'] in descendants}
        if expanded == descendants:
            break
        descendants = expanded
    # VMMs may still be children during startup; never count their RSS twice.
    return [rows[pid] for pid in sorted(descendants - set(vm_pids)) if pid in rows]


class Observations:
    def __init__(self, root, services, cgroup_parent):
        self.root, self.services, self.cgroup_parent = root, services, cgroup_parent
        self.refresh_processes()
        self.phase, self.rows, self.live = 'startup', [], []
        self.stopped = asyncio.Event()

    def refresh_processes(self):
        # Planned fixture restarts are observed explicitly. The sampler never
        # accepts a silently substituted service process or omits Redis/center.
        facts = self.services.processes.observe()
        self.processes = [(self.services.processes.children[name], value['start_time_ticks'])
                          for name, value in facts.items()]

    def snapshot(self):
        groups = []
        for vm in list(self.live):
            group = self.cgroup_parent / vm['resource_id']
            try:
                assert group.stat().st_ino == vm['cgroup_inode']
                groups.append({'resource_id': vm['resource_id'],
                               'memory_current': int((group / 'memory.current').read_text()),
                               'memory_peak': int((group / 'memory.peak').read_text()),
                               'memory_events': dict(line.split() for line in
                                                     (group / 'memory.events').read_text().splitlines())})
            except FileNotFoundError:
                # Destruction can race a periodic sample. Deterministic live
                # checkpoints and the final physical audit separately assert it.
                pass
        rows = {}
        for process, ticks in self.processes:
            assert process.poll() is None
            for row in process_tree(process.pid, ticks, [vm['pid'] for vm in self.live]):
                rows[row['pid']] = row
        tree = list(rows.values())
        return {'monotonic_seconds': time.monotonic(), 'phase': self.phase,
                'service_and_relay_processes': tree,
                'service_and_relay_rss_bytes': sum(row['rss_bytes'] for row in tree),
                'vmm_memory_current_bytes': sum(row['memory_current'] for row in groups),
                'vmm_cgroups': groups, 'host': host_headroom(self.root)}

    async def mark(self, phase):
        self.phase = phase
        row = await asyncio.to_thread(self.snapshot)
        row['checkpoint'] = True
        self.rows.append(row)

    async def sample(self):
        while not self.stopped.is_set():
            self.rows.append(await asyncio.to_thread(self.snapshot))
            try:
                await asyncio.wait_for(self.stopped.wait(), timeout=0.1)
            except TimeoutError:
                pass


async def live_shape(root, args, sandbox, native_id):
    _, name, record = record_for(root, native_id=native_id)
    assert process_start_ticks(record['pid']) == record['start_time_ticks']
    group = args.cgroup_parent / name
    bound = json.loads((original_chroot(root) / '.claims' / name / 'cgroup.json').read_text())
    assert group.stat().st_ino == bound['inode']
    assert Path('/proc', str(record['pid']), 'cgroup').read_text().strip() == (
        '0::/' + str(group.relative_to('/sys/fs/cgroup')))
    limits = {key: (group / key).read_text().strip() for key in
              ('cpu.max', 'cpu.max.burst', 'memory.max', 'memory.swap.max', 'memory.oom.group')}
    assert limits == {'cpu.max': '100000 100000', 'cpu.max.burst': '0',
                      'memory.max': str(1024 * MIB), 'memory.swap.max': '0', 'memory.oom.group': '1'}
    private = original_chroot(root) / args.firecracker.name / name / 'root/rootfs.ext4'
    disk = private.stat()
    source = args.rootfs.stat()
    assert disk.st_size == source.st_size
    assert (disk.st_dev, disk.st_ino) != (source.st_dev, source.st_ino)
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(str(private.parent / 'api.socket'))
        connection.sendall(b'GET /machine-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
        with http.client.HTTPResponse(connection) as response:
            response.begin()
            assert response.status == 200
            machine = json.load(response)
    assert (machine['vcpu_count'], machine['mem_size_mib']) == (1, 512)
    info = await call(sandbox.get_info)
    assert (info.cpu_count, info.memory_mb) == (1, 512)
    network = json.loads((original_chroot(root) / '.claims' / name / 'network.json').read_text())
    return {'sandbox_id': sandbox.sandbox_id, 'native_id': native_id, 'resource_id': name, 'pid': record['pid'],
            'start_ticks': record['start_time_ticks'], 'network_inode': network['namespace_inode'],
            'cgroup_inode': bound['inode'], 'limits': limits,
            'disk': {'device': disk.st_dev, 'inode': disk.st_ino, 'bytes': disk.st_size},
            'machine': {'cpus': machine['vcpu_count'], 'memory_mib': machine['mem_size_mib']}}


def audit_gone(root, args, rows):
    for row in rows:
        name = row['resource_id']
        assert process_start_ticks(row['pid']) != row['start_ticks']
        assert not (original_chroot(root) / args.firecracker.name / name).exists()
        assert not Path('/run/netns', name).exists()
        assert not (args.cgroup_parent / name).exists()
    inodes = {row['network_inode'] for row in rows}
    for proc in Path('/proc').iterdir():
        if proc.name.isdecimal():
            try:
                assert (proc / 'ns/net').stat().st_ino not in inodes
            except (FileNotFoundError, ProcessLookupError):
                pass


async def exercise(args):
    assert sys.platform == 'linux' and os.geteuid() == 0
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert 1000 <= args.uid_base and args.uid_base + VM_COUNT < 2**32
    assert not processes_in_uid_range(args.uid_base, args.uid_base + VM_COUNT - 1)
    checkout = Path(__file__).resolve().parents[2]
    assert not subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain'])
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    for name in ('center', 'host', 'jailer', 'firecracker', 'kernel', 'rootfs', 'scratch_parent', 'cgroup_parent'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    parent = args.scratch_parent
    assert parent.stat().st_uid == 0 and not parent.stat().st_mode & 0o077
    assert not (args.cgroup_parent / 'cgroup.procs').read_text().strip()
    assert not any(path.is_dir() for path in args.cgroup_parent.iterdir())
    assert {'cpu', 'memory'} <= set((args.cgroup_parent / 'cgroup.subtree_control').read_text().split())
    budgets = json.loads((checkout / 'benchmarks/local-service-budgets.json').read_text())
    preflight = host_headroom(parent)
    assert preflight['memory_available_bytes'] >= 6 * 1024 * MIB
    assert len(os.sched_getaffinity(0)) >= 3
    assert preflight['disk_available_bytes'] >= 3 * args.rootfs.stat().st_size + budgets['cleanup_reserve_bytes']
    inputs = {name: {'sha256': sha256_file(getattr(args, name)), 'bytes': getattr(args, name).stat().st_size}
              for name in ('center', 'host', 'jailer', 'firecracker', 'kernel', 'rootfs')}
    before_network = network_state()
    root = Path(tempfile.mkdtemp(prefix='tp-', dir=parent))
    namespace = secrets.token_hex(8)
    assert len(os.fsencode(root / 'p/v' / args.firecracker.name / f'vm-{namespace}-5/root/api.socket')) < 108
    print(json.dumps({'event': 'run_created', 'root': str(root), 'namespace': namespace,
                      'source_commit': commit}), flush=True)
    for name in ('db', 'state', 'p'):
        (root / name).mkdir(mode=0o700)
    keys = [secrets.token_hex(32), secrets.token_hex(32)]
    url = f'http://127.0.0.1:{free_loopback_port()}'
    database = root / 'db/controller.db'
    template = descriptor('runtime-base', args.kernel, args.rootfs, 1, 512)
    template['owner'] = None
    template['resources']['host_memory_bytes'] = 1024 * MIB
    catalog = dict(templates=[template], aliases={'base': template['template_id']}, compute_limits=dict(
        max_vcpus_global=3, max_vcpus_per_tenant=2,
        max_memory_bytes_global=3*1024*MIB, max_memory_bytes_per_tenant=2*1024*MIB))
    settings = {**budgets, 'namespace': namespace, 'database': str(database),
                'node_state_dir': str(root / 'state'), 'jailer': str(args.jailer),
                'firecracker': str(args.firecracker), 'chroot_base': str(root / 'p/v'),
                'kernel': str(args.kernel), 'rootfs': str(args.rootfs), 'uid_base': args.uid_base,
                'vcpu_count': 1, 'memory_mib': 512,
                'host_resources': {'cgroup_parent': str(args.cgroup_parent), 'memory_max_bytes': 1024 * MIB},
                'file_policy_digest': 1, 'templates': catalog, 'public_ipv4_enabled': False,
                'max_active_global': 3, 'max_active_per_tenant': 2, 'startup_timeout_secs': 120}
    settings = complete_runtime(settings, slots=3)
    registry = dict(root=str(root / 'registry'), max_templates_global=1, max_templates_per_tenant=1,
        max_artifact_bytes_global=args.kernel.stat().st_size+args.rootfs.stat().st_size,
        max_artifact_bytes_per_tenant=args.kernel.stat().st_size+args.rootfs.stat().st_size,
        validation_timeout_secs=60)
    services = RuntimeServices.prepare(args, root, [settings], [catalog], keys[0],
        url.removeprefix('http://'), args.service_cgroup_record, job_count=12, registry_config=registry,
        extra_credentials=[dict(tenant=2, secret=keys[1])])
    sampler, monitor = None, None
    known, evidence, results = [], [], []
    completed = False
    try:
        services.start()
        await asyncio.to_thread(services.wait_ready)
        for cls in (Sandbox, AsyncSandbox):
            cohort = []
            for tenant in (0, 0, 1):
                owner = options(keys[tenant], url, url)
                owner['request_timeout'] = 120
                sandbox = await call(cls.create, template='base', timeout=600,
                                     allow_internet_access=False, **owner)
                known.append(sandbox)
                cohort.append(sandbox)
            # The global IDs cannot identify a local VM. Stop this exact host
            # once for its original offline inbox audit; then restore its SAME
            # configuration before the measured workload, without initialization.
            services.processes.stop('host-0')
            native_ids = [services.native_id_for(item.sandbox_id) for item in cohort]
            services.processes.start('host-0')
            await asyncio.to_thread(services.wait_ready, for_admission=False)
            monitor = Observations(root, services, args.cgroup_parent) if monitor is None else monitor
            monitor.refresh_processes()
            sampler = asyncio.create_task(monitor.sample())
            for sandbox, native_id in zip(cohort, native_ids):
                row = await live_shape(root, args, sandbox, native_id)
                evidence.append(row)
                monitor.live.append(row)
            assert len({(row['disk']['device'], row['disk']['inode']) for row in monitor.live}) == 3
            async def observe(phase):
                await monitor.mark(cls.__name__ + ':' + phase)
            await observe('three-vms-ready')
            results.append({'sdk': cls.__name__, **await pressure(cohort, url, budgets['data_transfer'], observe)})
            assert [await live_shape(root, args, sandbox, native_id)
                    for sandbox, native_id in zip(cohort, native_ids)] == monitor.live
            for sandbox in cohort:
                _kill_confirmed = (await call(sandbox.kill))
                assert _kill_confirmed
                known.remove(sandbox)
            audit_gone(root, args, monitor.live)
            monitor.live = []
            await observe('cohort-destroyed')
            monitor.stopped.set()
            await sampler
            sampler = None
            monitor.stopped = asyncio.Event()
        assert len(evidence) == VM_COUNT
        assert all(terminal_cleanup_confirmed(database, row['native_id']) for row in evidence)
        audit_gone(root, args, evidence)
        assert not processes_in_uid_range(args.uid_base, args.uid_base + 2)
        assert not any(path.is_dir() for path in args.cgroup_parent.iterdir())
        assert network_state() == before_network
        assert inputs == {name: {'sha256': sha256_file(getattr(args, name)), 'bytes': getattr(args, name).stat().st_size}
                          for name in inputs}
        assert not subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain'])
        full = [row for row in monitor.rows if row.get('checkpoint') and row['phase'].endswith(':global-full')]
        assert len(full) == 2 and all(len(row['vmm_cgroups']) == 3 for row in full)
        assert all(int(group['memory_events'].get(event, 0)) == 0 for row in monitor.rows
                   for group in row['vmm_cgroups'] for event in ('oom', 'oom_kill'))
        report = {'source_commit': commit, 'probe_sha256': sha256_file(Path(__file__)),
                  'sdk_probe_sha256': sha256_file(Path(__file__).with_name('sdk_task_global_pressure.py')),
                  'inputs': inputs, 'host_kernel': os.uname().release, 'sdk': '2.51.0',
                  'preflight': preflight, 'task_budgets': budgets,
                  'configuration_overrides': {key: settings[key] for key in
                      ('max_active_global', 'max_active_per_tenant', 'startup_timeout_secs', 'vcpu_count', 'memory_mib', 'host_resources')},
                  'results': results, 'vms': evidence, 'vms_destroyed': VM_COUNT,
                  'shared_network_unchanged': True, 'inputs_unchanged': True,
                  'observations': monitor.rows,
                  'scope': 'Fixed task transfer budgets; three-VM/two-tenant sync/async admission and concurrent '
                           '8/64 MiB hash-checked transfers. VM admission is explicitly raised from serial. '
                           '100 ms nominal sampling plus barriers; sampled RSS sums share pages and are not PSS, '
                           'a strict peak or a whole-host cap. Host headroom includes unrelated processes. '
                           'No long idle deadline, full task rerun, final production budget or proof claim.'}
        (root / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        completed = True
        print(json.dumps({'event': 'passed', 'source_commit': commit, 'root': str(root),
                          'vms_destroyed': VM_COUNT, 'samples': len(monitor.rows)}), flush=True)
    finally:
        cleanup_errors = []
        if monitor:
            monitor.stopped.set()
        if sampler:
            outcomes = await asyncio.gather(sampler, return_exceptions=True)
            cleanup_errors.extend(item for item in outcomes if isinstance(item, BaseException))
        try:
            # Unknown create results are retained for exact database/host
            # recovery by the operator. Never blindly create a replacement.
            for sandbox in known:
                try:
                    _kill_confirmed = (await call(sandbox.kill))
                    assert _kill_confirmed
                except Exception as error:
                    cleanup_errors.append(error)
        finally:
            services.stop_all()
        if completed and not args.keep_artifacts:
            services.cleanup_staging()
            shutil.rmtree(root)
        else:
            print(f'Exact run artifacts retained: {root}', file=sys.stderr)
        if cleanup_errors:
            raise BaseExceptionGroup('probe cleanup or resource sampling failed', cleanup_errors)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    for argument in ('jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent', 'cgroup-parent'):
        parser.add_argument(f'--{argument}', type=Path, required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    parser.add_argument('--keep-artifacts', action='store_true')
    asyncio.run(exercise(parser.parse_args()))
