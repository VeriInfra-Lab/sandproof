"""Outer KVM supervisor for two finite unchanged-profile gateway cases.

Only two new deny-all VMs, a new private run root and an operator-delegated
service leaf are owned. No shared network/cgroup configuration, broad process
killing, database deletion, production budget change or proof claim is made.
Failure retains private evidence and any resource whose exact cleanup failed.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
from contextlib import closing
import hashlib
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

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import check_kvm_host as host
from tools import service_cgroup

MIB = 1024 * 1024
VM_COUNT = 2
SERVICE_MEMORY = 1024 * MIB
VMM_MEMORY = 768 * MIB


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_new(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def service_groups(runtime, services):
    runtime, services = Path(runtime), Path(services)
    for path in (runtime, services):
        host.trusted_path(path)
        service_cgroup.filesystem(path)
        if any(child.is_dir() for child in path.iterdir()):
            raise ValueError('exclusive empty delegated cgroup parent required')
    if runtime == services or runtime in services.parents or services in runtime.parents:
        raise ValueError('runtime and service cgroup parents must be disjoint')


def configuration(budgets, root, namespace, artifacts, *, uid, cgroup, base, file_size=None):
    settings = dict(budgets)
    settings.update(namespace=namespace, database=str(root / 'db/controller.db'),
        node_state_dir=str(root / 'state'), chroot_base=str(root / 'p/v'),
        **{key: spec['path'] for key, spec in artifacts.items()},
        uid_base=uid, vcpu_count=1, memory_mib=512,
        host_resources={'cgroup_parent': str(cgroup), 'memory_max_bytes': VMM_MEMORY},
        file_policy_digest=1,
        max_active_global=2, max_active_per_tenant=1, startup_timeout_secs=120,
        public_ipv4_enabled=False,
        templates={'templates': [base], 'aliases': {'base': base['template_id']},
                   'compute_limits': {'max_vcpus_global': 2, 'max_vcpus_per_tenant': 1,
                       'max_memory_bytes_global': 2 * VMM_MEMORY,
                       'max_memory_bytes_per_tenant': VMM_MEMORY}})
    if settings['data_transfer'] != budgets['data_transfer']:
        raise ValueError('final transfer budgets changed')
    longest = root / 'p/v' / Path(artifacts['firecracker']['path']).name / f'vm-{namespace}-1/root/api.socket'
    if len(os.fsencode(longest)) >= 108:
        raise ValueError('owned Firecracker socket path exceeds Linux limit')
    complete_runtime = _runpy.run_path(
        str(Path(__file__).with_name('_complete_runtime.py'))
    )['complete_runtime']
    return complete_runtime(settings, slots=VM_COUNT, file_size=file_size)


def assert_no_oom(baseline, snapshot):
    if snapshot['identity'] != baseline['identity'] or snapshot['limits'] != baseline['limits']:
        raise AssertionError('service cgroup identity or limits changed')
    maximum = int(baseline['limits']['memory.max'])
    if snapshot['memory_peak'] > maximum or snapshot['memory_current'] > maximum:
        raise AssertionError('service hard memory ceiling violated')
    for name in ('oom', 'oom_kill', 'oom_group_kill'):
        if snapshot['memory_events'].get(name, 0) != baseline['memory_events'].get(name, 0):
            raise AssertionError('service OOM event increased')


def kill_each(instances, call):
    """A failed DELETE never prevents the other known identity's DELETE."""
    outcomes = []
    for instance in instances:
        try:
            confirmed = call(instance.kill)
            if not confirmed:
                raise AssertionError('known sandbox DELETE was not confirmed')
            outcomes.append({'sandbox_id': instance.sandbox_id, 'confirmed': True})
        except Exception as error:
            outcomes.append({'sandbox_id': instance.sandbox_id, 'confirmed': False,
                             'failure_type': type(error).__name__})
    return outcomes


def wait_service_empty(record, baseline, *, timeout=10, clock=time.monotonic, sleep=time.sleep, evidence=None):
    deadline = clock() + timeout
    while True:
        snapshot = service_cgroup.observe(record)
        if evidence is not None:
            evidence.clear()
            evidence.update(snapshot)  # Preserve counters even when the next check fails.
        assert_no_oom(baseline, snapshot)
        if not snapshot['pids'] and not snapshot['populated']:
            return snapshot
        if clock() >= deadline:
            raise AssertionError('owned service leaf retained live processes')
        sleep(0.05)  # Wait for the exact observed leaf, never kill guessed PIDs.


def live_identity(root, args, sandbox, *, native_id, local_number):
    from server_process_crash import process_start_ticks
    from server_templates import record_for
    _secret, name, claim = record_for(root, native_id=native_id)
    if process_start_ticks(claim['pid']) != claim['start_time_ticks']:
        raise AssertionError('original VMM PID/start identity changed')
    status = Path(f'/proc/{claim["pid"]}/status').read_text().splitlines()
    uid = next(line.split()[1:] for line in status if line.startswith('Uid:'))
    with closing(sqlite3.connect(f'file:{root / "db/controller.db"}?mode=ro', uri=True)) as connection:
        contexts = [json.loads(blob) for (blob,) in connection.execute(
            'SELECT context_json FROM controller_host_slots')]
    context = next(value for value in contexts
                   if value['binding']['sandbox'] == local_number)
    if context['binding']['owner'] not in (1, 2) or context['binding']['vm'] != int(name.rsplit('-', 1)[1]):
        raise AssertionError('original sandbox physical binding differs')
    expected_uid = args.uid_base + context['position']
    if [int(value) for value in uid] != [expected_uid] * 4:
        raise AssertionError('actual VMM UID differs from its exclusive configured identity')
    group = args.cgroup_parent / name
    cgroup = json.loads((original_chroot(root) / '.claims' / name / 'cgroup.json').read_bytes())
    network = json.loads((original_chroot(root) / '.claims' / name / 'network.json').read_bytes())
    group_identity = group.stat()
    if (group_identity.st_dev, group_identity.st_ino) != (cgroup['device'], cgroup['inode']):
        raise AssertionError('original VMM cgroup identity changed')
    if Path(f'/proc/{claim["pid"]}/cgroup').read_text().strip() != '0::/' + str(group.relative_to('/sys/fs/cgroup')):
        raise AssertionError('VMM outside original runtime group')
    limits = {key: (group / key).read_text().strip() for key in (
        'cpu.max', 'cpu.max.burst', 'memory.max', 'memory.swap.max', 'memory.oom.group')}
    if limits != {'cpu.max': '100000 100000', 'cpu.max.burst': '0',
                  'memory.max': str(VMM_MEMORY), 'memory.swap.max': '0', 'memory.oom.group': '1'}:
        raise AssertionError('VMM configured shape differs')
    events = service_cgroup.counters(group / 'memory.events')
    if any(events.get(name, 0) for name in ('oom', 'oom_kill', 'oom_group_kill')):
        raise AssertionError('new VMM had an OOM event')
    namespace = Path('/run/netns', name).stat()
    if (namespace.st_ino != network['namespace_inode']
            or network.get('namespace_device') not in (None, namespace.st_dev)):
        raise AssertionError('original VM network namespace changed')
    private = original_chroot(root) / args.firecracker.name / name / 'root/rootfs.ext4'
    disk, source = private.stat(), args.rootfs.stat()
    if disk.st_size != source.st_size or (disk.st_dev, disk.st_ino) == (source.st_dev, source.st_ino):
        raise AssertionError('VM disk is not a full independent base copy')
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(str(private.parent / 'api.socket'))
        connection.sendall(b'GET /machine-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
        with http.client.HTTPResponse(connection) as response:
            response.begin()
            if response.status != 200:
                raise AssertionError('real Firecracker machine inspection failed')
            machine = json.load(response)
    if (machine['vcpu_count'], machine['mem_size_mib']) != (1, 512):
        raise AssertionError('actual Firecracker compute shape differs')
    info = sandbox.get_info()
    if (info.cpu_count, info.memory_mb) != (1, 512):
        raise AssertionError('SDK shape differs from real Firecracker')
    return {'sandbox_id': sandbox.sandbox_id, 'native_id': native_id, 'resource_id': name, 'pid': claim['pid'],
            'start_ticks': claim['start_time_ticks'], 'uid': expected_uid,
            'network_device': namespace.st_dev, 'network_inode': namespace.st_ino,
            'cgroup_device': cgroup['device'], 'cgroup_inode': cgroup['inode'], 'limits': limits,
            'disk': {'device': disk.st_dev, 'inode': disk.st_ino, 'bytes': disk.st_size}}


def namespace_holders(device, inode):
    """Fail closed on unknown inspection; never kill an attributed/unknown holder."""
    holders = []
    for process in Path('/proc').iterdir():
        if not process.name.isdecimal():
            continue
        try:
            paths = [process / 'ns/net', *list((process / 'fd').iterdir())]
            for path in paths:
                try:
                    meta = path.stat()
                    if (meta.st_dev, meta.st_ino) == (device, inode):
                        holders.append(int(process.name))
                        break
                except (FileNotFoundError, ProcessLookupError):
                    pass  # A vanished process/FD cannot retain this original namespace.
        except (FileNotFoundError, ProcessLookupError):
            pass
    return sorted(holders)


def check_child_report(path, code, settings_sha, server_sha, identities, *, host_config_sha,
                       command_output_only=False):
    value = json.loads(path.read_bytes(), object_pairs_hook=host.unique_keys)
    if (code != 0 or value.get('result') != 'passed' or value.get('config_sha256') != settings_sha
            or value.get('server', {}).get('executable_sha256') != server_sha
            or value.get('host_config_sha256') != host_config_sha
            or value.get('sandbox_ids') != identities
            or set(value.get('cases', {})) != (
                {'command_output_sync', 'command_output_async', 'template_start_once'}
                if command_output_only else {'file_idle_and_progress', 'paused_download_memory'})):
        raise AssertionError('original child acceptance report incomplete or mismatched')
    return value


def exercise(args, report):
    from e2b import Sandbox
    from sdk_c02_acceptance import processes_in_uid_range
    from sdk_server_smoke import options
    from server_process_crash import free_loopback_port, process_start_ticks, terminal_cleanup_confirmed
    from _runtime_services import RuntimeServices
    from server_task_pressure import audit_gone, network_state
    from server_templates import descriptor
    if importlib.metadata.version('e2b') != '2.51.0':
        raise ValueError('fixed original SDK 2.51.0 required')
    report['source'] = host.source_revision()
    report['probe_sha256'] = digest(__file__)
    report['gateway_probe_sha256'] = digest(ROOT / 'tests/system/gateway_slow_boundaries.py')
    report['host_kernel'] = os.uname().release
    for role in ('center', 'host'):
        host.trusted_path(getattr(args, role))
        if not getattr(args, role).is_file():
            raise ValueError('approved product binary required')
    parent = host.trusted_path(args.scratch_parent)
    service_cgroup.private(parent)
    host.trusted_path(args.cgroup_parent)
    service_cgroup.filesystem(args.cgroup_parent)
    if any(child.is_dir() for child in args.cgroup_parent.iterdir()):
        raise ValueError('exclusive empty runtime cgroup parent required')
    if not 1000 <= args.uid_base < 2**32 - VM_COUNT or processes_in_uid_range(args.uid_base, args.uid_base + VM_COUNT - 1):
        raise ValueError('exclusive usable UID range required')
    approved, approved_sha = host.load_config(host.trusted_path(args.host_config))
    for key in host.ARTIFACTS:
        setattr(args, key, Path(approved['artifacts'][key]['path']))
    budgets_path = ROOT / 'benchmarks/local-service-budgets.json'
    budgets = json.loads(budgets_path.read_bytes())
    report['final_budget_sha256'] = digest(budgets_path)
    report['data_transfer'] = budgets['data_transfer']
    inputs = {key: {'sha256': digest(getattr(args, key)), 'bytes': getattr(args, key).stat().st_size}
              for key in (*host.ARTIFACTS, 'center', 'host')}
    report['inputs'] = inputs
    before_network = network_state()
    root = Path(tempfile.mkdtemp(prefix='gs-', dir=parent))
    report['run_root'] = str(root)
    namespace = secrets.token_hex(8)
    report['namespace'], report['uid_base'] = namespace, args.uid_base
    for child in ('db', 'state', 'p'):
        (root / child).mkdir(mode=0o700)
    preflight = dict(approved, chroot_base=str(root / 'p/v'),
                     min_free_disk_bytes=max(approved['min_free_disk_bytes'],
                                             3 * inputs['rootfs']['bytes'] + budgets['cleanup_reserve_bytes']),
                     min_available_memory_bytes=max(approved['min_available_memory_bytes'],
                                                    2 * VMM_MEMORY + SERVICE_MEMORY + 1024 * MIB))
    preflight_path = root / 'host-preflight.json'
    write_new(preflight_path, preflight)
    report['approved_host_config_sha256'] = approved_sha
    report['host_preflight'] = host.inspect_host(preflight, digest(preflight_path))
    if not report['host_preflight']['prerequisites_passed']:
        raise ValueError('exact reviewed host preflight incomplete')
    keys = [secrets.token_hex(32), secrets.token_hex(32)]
    url = f'http://127.0.0.1:{free_loopback_port()}'
    base = descriptor('slow-boundary-base', args.kernel, args.rootfs, 1, 512)
    base['owner'] = None
    if args.command_output_only:
        # First-frame observation must detach only curl, not this guest process.
        base['startup'] = {'user': 'root', 'workdir': '/', 'env_vars': {},
                           'argv': ['/bin/sh', '-c', "printf 'once\\n' >> /tmp/template-starts; exec sleep 600"]}
    settings = configuration(budgets, root, namespace, approved['artifacts'], uid=args.uid_base,
                             cgroup=args.cgroup_parent, base=base)
    registry = {'root': str(root / 'registry'), 'max_templates_global': 1,
        'max_templates_per_tenant': 1, 'max_artifact_bytes_global': sum(inputs[k]['bytes'] for k in ('kernel','rootfs')),
        'max_artifact_bytes_per_tenant': sum(inputs[k]['bytes'] for k in ('kernel','rootfs')),
        'validation_timeout_secs': 60}
    services = RuntimeServices.prepare(args, root=root, native_pools=[settings],
        registered_catalogs=[settings['templates']], key=keys[0], public_address=url.removeprefix('http://'),
        record=args.service_cgroup_record, job_count=4, registry_config=registry,
        extra_credentials=[{'tenant': 2, 'secret': keys[1]}])
    settings = services.native_pools[0]
    config = services.processes.bindings['center']['config']
    host_config = services.processes.bindings['host-0']['config']
    cgroup_record = args.service_cgroup_record
    report['config_sha256'], report['host_config_sha256'] = digest(config), digest(host_config)
    baseline = service_cgroup.observe(cgroup_record)
    if baseline['pids'] or baseline['populated'] or int(baseline['limits']['memory.max']) != SERVICE_MEMORY:
        raise ValueError('exclusive unchanged one-GiB service leaf required')
    report['service_limits'] = baseline['limits']
    report['service_before'] = baseline
    known, rows, process, ticks = [], [], None, None
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        services.start()
        services.wait_ready()
        for key in keys:
            sandbox = Sandbox.create(template='base', timeout=600, allow_internet_access=False,
                                     **(options(key, url, url) | {'request_timeout': 120}))
            known.append(sandbox)  # Own the successful identity before any further inspection.
        services.processes.stop('host-0')
        native_ids = [services.native_id_for(sandbox.sandbox_id) for sandbox in known]
        local_numbers = [services.local_for(sandbox.sandbox_id) for sandbox in known]
        services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        process = services.processes.children['center']
        ticks = process_start_ticks(process.pid)
        report['server_identity'] = {'pid': process.pid, 'start_ticks': ticks}
        report['service_started'] = service_cgroup.observe(cgroup_record,
            pids=[child.pid for child in services.processes.children.values()])
        rows = [live_identity(root, args, sandbox, native_id=native_id, local_number=number)
                for sandbox, native_id, number in zip(known,native_ids,local_numbers,strict=True)]
        if len({(row['disk']['device'], row['disk']['inode']) for row in rows}) != VM_COUNT:
            raise AssertionError('the two VM disks share an identity')
        report['vms_before'] = rows
        child_report = root / 'gateway-report.json'
        command = [sys.executable, str(ROOT / 'tests/system/gateway_slow_boundaries.py'),
                   '--api-url', url, '--sandbox-url', url,
                   '--target-sandbox-id', known[0].sandbox_id, '--control-sandbox-id', known[1].sandbox_id,
                   '--config', str(config), '--host-config', str(host_config),
                   '--center-uid', str(args.center_uid), '--center-gid', str(args.center_gid),
                   '--service-cgroup-record', str(cgroup_record),
                   '--server-pid', str(process.pid), '--server-start-ticks', str(ticks), '--report', str(child_report)]
        if args.command_output_only:
            command.append('--command-output-only')
        environment = {'PATH': '/usr/bin:/bin', 'LANG': 'C',
                       'SANDPROOF_TEST_API_KEY': keys[0], 'SANDPROOF_TEST_FOREIGN_API_KEY': keys[1]}
        with (root / 'gateway.stdout').open('xb') as out, (root / 'gateway.stderr').open('xb') as err:
            out_path, err_path = Path(out.name), Path(err.name)
            out_path.chmod(0o600)
            err_path.chmod(0o600)
            child = subprocess.run(command, env=environment, stdout=out, stderr=err, timeout=420)
        report['gateway_returncode'] = child.returncode
        child_value = check_child_report(child_report, child.returncode, report['config_sha256'],
                                         inputs['center']['sha256'], [s.sandbox_id for s in known],
                                         host_config_sha=report['host_config_sha256'],
                                         command_output_only=args.command_output_only)
        report['gateway'] = {'report_sha256': digest(child_report), 'result': child_value['result'],
                             'cases': child_value['cases']}
        if [live_identity(root, args, sandbox, native_id=native_id, local_number=number)
            for sandbox,native_id,number in zip(known,native_ids,local_numbers,strict=True)] != rows:
            raise AssertionError('a VM identity changed during accepted gateway work')
        if (process_start_ticks(process.pid) != ticks or digest(config) != report['config_sha256']
                or digest(host_config) != report['host_config_sha256']):
            raise AssertionError('original server/config changed')
        snapshot = service_cgroup.observe(cgroup_record, pids=[process.pid])
        assert_no_oom(baseline, snapshot)
        report['service_after_cases'] = snapshot
    finally:
        report['destroy_results'] = kill_each(known, lambda method: method(request_timeout=40))
        errors = [row['failure_type'] for row in report['destroy_results'] if not row['confirmed']]
        for row in rows:  # One failed audit cannot suppress the other exact-resource audit.
            try:
                audit_gone(root, args, [row])
                if namespace_holders(row['network_device'], row['network_inode']):
                    raise AssertionError('original namespace identity still has a process/FD holder')
            except Exception as error:
                errors.append(type(error).__name__)
        try:
            services.stop_all()
        except Exception as error:
            errors.append(type(error).__name__)
        try:
            report['service_after_exit'] = {}
            wait_service_empty(cgroup_record, baseline, evidence=report['service_after_exit'])
            report['service_removed'] = 'operator-owned leaf preserved; all owned roles observed empty'
        except Exception as error:
            errors.append(type(error).__name__)
        report['cleanup_errors'] = errors
        if errors:
            raise AssertionError('exact VM/service cleanup incomplete; retain original run evidence')
    if processes_in_uid_range(args.uid_base, args.uid_base + VM_COUNT - 1):
        raise AssertionError('owned UID range retained a process')
    if any(p.is_dir() for p in args.cgroup_parent.iterdir()):
        raise AssertionError('owned runtime cgroup parent retained a VM group')
    if not all(terminal_cleanup_confirmed(settings['database'], row['native_id']) for row in rows):
        raise AssertionError('not all accepted VM records are terminal or exactly retired')
    if network_state() != before_network:
        raise AssertionError('shared host network changed after exact cleanup')
    if inputs != {key: {'sha256': digest(getattr(args, key)), 'bytes': getattr(args, key).stat().st_size} for key in inputs}:
        raise AssertionError('approved host inputs changed')
    if host.source_revision() != report['source']:
        raise AssertionError('approved source changed during acceptance')
    report['host_cleanup'] = 'two_original_vms_and_service_leaf_verified_gone'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    from _runtime_services import add_arguments
    add_arguments(parser)
    for field in ('host-config', 'scratch-parent', 'cgroup-parent'):
        parser.add_argument('--' + field, required=True, type=Path)
    parser.add_argument('--uid-base', required=True, type=int)
    parser.add_argument('--command-output-only', action='store_true',
                        help='finite sync/async large output and template Start-prefix check instead of slow files')
    args = parser.parse_args(argv)
    if sys.platform != 'linux' or os.geteuid() != 0:
        parser.error('exclusive root Linux/KVM host required')
    report = {'schema_version': 1, 'result': 'incomplete', 'cleanup_errors': [],
              'scope': 'two_finite_final_profile_sync_gateway_cases_with_real_host_cleanup',
              'not_tested': ['13000_second_idle_expiry', 'complete_terminal_bench_suite', 'formal_proofs']}
    if args.command_output_only:
        report['scope'] = 'finite_sync_async_large_output_and_template_start_with_real_host_cleanup'
        report['not_tested'].append('slow_file_cases')
    else:
        report['not_tested'].append('async_sdk')
    code = 1
    try:
        exercise(args, report)
        report['result'], code = 'passed', 0
    except Exception as error:
        report['failure_type'] = type(error).__name__
    finally:
        if 'run_root' in report:
            path = Path(report['run_root']) / 'report.json'
            write_new(path, report)
            print(json.dumps({'result': report['result'], 'report': str(path), 'report_sha256': digest(path)}))
        else:
            print(json.dumps({'result': 'incomplete', 'failure_type': report.get('failure_type')}))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
