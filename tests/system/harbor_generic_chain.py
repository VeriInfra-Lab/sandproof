"""Isolated KVM acceptance of original tasks via self-service templates.

Template builds use two independent pools; prepared CI runs only the runtime
pool, with no build/publication authority. Both use exclusive namespaces/UIDs.
Retains original failures and audit evidence. No manual task recipes/models.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from tools.template_build_config import acceptance_build_policy, apply_acceptance_build_policy
from tools.bounded_acceptance import (
    acceptance_disk_budgets, bounded_acceptance_layout, completion_build_observation,
    provision_acceptance_coordination, require_bounded_host_tools, validate_acceptance_socket_paths,
    prepared_ci_physical_profile, prepared_ci_runtime_configuration,
)

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
import time
import urllib.request
from types import SimpleNamespace

from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from server_process_crash import fetch, free_loopback_port, process_start_ticks
from _process_utils import stop_child
from _runtime_services import RuntimeServices, add_arguments
from _center_host_services import find_runtime_create, terminal_result, process_facts
from tools.runtime_create_recovery import recover_runtime_create
from server_templates import descriptor, record_for
from server_template_isolation import audit_gone
from server_vm_id_capacity import check_no_resources
from template_self_service import BUILD_TOOLS
from template_resource_pressure import ready_for_pressure
from template_build import IGZIP_HASHES
from tools.benchmark_tasks import verify
from tools.generic_capacity import EXPANDED_BYTES, INPUT_BYTES, plan
from tools.benchmark_proxy import BenchmarkProxy
from tools.template_acceptance import budgets, check_headroom, load, PROFILE
from tools import service_cgroup
from tools.system_acceptance import write_once,json_file
from benchmarks.artifact_budget import add_artifact_arguments, artifact_limits_from_args
from benchmarks.check_harbor import check_runtime, check_tasks
from benchmarks.run_generic import run_selected, validate_oracle_mode
from benchmarks.environment_repairs import PRUNED_SOURCE, SOURCE_KINDS, validate_source_kind

MIB = 1024**2


def network_transport_evidence(transport, *, prepared=False):
    if prepared:
        if transport is not None:
            raise ValueError('prepared CI cannot report a live outbound transport as disabled')
        return {'mode': 'disabled', 'public_network': False}
    return transport.evidence() if transport else {'mode': 'direct_public_ipv4'}


def validate_build_service(path, *, prepared=False, require_physical_lease=True):
    # Prepared fixtures are registered by the enclosing runtime harness. They
    # never authorize a build, so do not validate an unused public builder.
    if prepared:
        return None
    from tools.template_build_service import configuration
    return configuration(path, require_physical_lease=require_physical_lease)


def launch_build_service(checkout, root, service_prefix, *, prepared=False):
    if prepared:
        return None, None
    config = root / 'build-control/build.json'
    validate_build_service(config)
    log = (root / 'control-logs/build-service.log').open('ab')
    try:
        process = subprocess.Popen([*service_prefix, sys.executable,
            str(checkout / 'tools/template_build_service.py'), '--config', str(config)],
            stdout=log, stderr=subprocess.STDOUT)
    except BaseException:
        log.close()
        raise
    return process, log


def pending_build_reservations(directory, *, prepared=False, row=None):
    if prepared:
        if row is not None and row.get('build_dispatched'):
            raise RuntimeError('prepared CI unexpectedly dispatched a template build')
        for name in ('builds.db', 'builds.db-wal', 'builds.db-shm', 'builds.db-journal'):
            path = directory / name
            if path.exists() or path.is_symlink():
                raise RuntimeError('prepared CI unexpectedly owns a build journal')
        return 0  # No build service exists; do not create a database to audit it.
    with contextlib.closing(sqlite3.connect(directory / 'builds.db')) as db:
        return db.execute('SELECT COUNT(*) FROM jobs WHERE reserved>0').fetchone()[0]


def repetition_outcome(rows, selected, *, oracle_mode, resource_pressure, pressure_report):
    """A separate pressure pass never replaces task scores or oracle provenance."""
    passed = (len(rows) == len(selected) and all(row['status'] == 'passed' for row in rows)
              and (not resource_pressure or pressure_report['status'] == 'passed'))
    return {'execution_passed': passed, 'oracle_mode': oracle_mode,
            'original_verifier': True, 'original_oracle_verifier': oracle_mode == 'original'}


def observe_service_resources(binary, service_record, pids, samples, output, *, minimum_relays=0):
    """Observe the configured relay protocol inside the original service budget."""
    snapshot = service_cgroup.observe(service_record, pids=pids)
    if snapshot['memory_events'].get('oom_kill', 0):
        raise RuntimeError('service cgroup OOM; retain original identities for recovery')
    relay_mode = b'relay'
    relays = []
    for pid in snapshot['pids']:
        try:
            start = process_start_ticks(pid)
            argv = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
            if start is not None and argv[:2] == [os.fsencode(binary), relay_mode]:
                service_cgroup.observe(service_record, pids=[pid])
                if process_start_ticks(pid) == start:
                    relays.append({'pid': pid, 'start_time_ticks': start})
        except FileNotFoundError:
            continue
    snapshot['relay_mode'] = relay_mode.decode('ascii')
    snapshot['relays'] = relays
    samples.append(snapshot)
    output.write_text(json.dumps(samples, indent=2))
    if len(relays) < minimum_relays:
        raise RuntimeError('blocked data relays were not observed inside the service budget')
    return snapshot


def ci_preparation_timeout(args):
    value = getattr(args, 'ci_preparation_timeout', None)
    if getattr(args, 'prepare_ci_fixtures', None) is None:
        if value is not None:
            raise ValueError('CI preparation timeout is only for one-time fixture preparation')
        return None
    from benchmarks.run_ci_prepare import validate_preparation_timeout
    return validate_preparation_timeout(value)


def select_cohort(args, tasks):
    """Keep execution/source validation separate from the capacity basis."""
    selected = {t['name'] for t in tasks} if args.all else set(args.task)
    if not selected or selected - {t['name'] for t in tasks}:
        raise RuntimeError('unknown original task')
    oracle_mode = validate_oracle_mode(args, selected)
    selected_entries = [t for t in tasks if t['name'] in selected]
    if getattr(args, 'ci_fixtures', None) is not None:
        if args.source_kind != 'prepared-ci' or oracle_mode != 'original':
            raise ValueError('prepared CI requires original oracle and explicit source mode')
    else:
        validate_source_kind(args.source_kind, selected_entries)
    if args.capacity_scope not in ('selected', 'all'):
        raise ValueError('unknown capacity scope')
    capacity_entries = list(tasks if args.capacity_scope == 'all' else selected_entries)
    preparation_timeout = ci_preparation_timeout(args)
    if preparation_timeout is not None:
        # Planning changes only the independent one-time builder deadline.
        # Original selected entries, resource shapes and the frozen lock stay intact.
        capacity_entries = [entry | {'requirements': entry['requirements'] |
                            {'build_timeout_sec': preparation_timeout}} for entry in capacity_entries]
    if args.resource_pressure:
        if len(selected) != 2 or args.repetitions != 1:
            raise ValueError('resource pressure requires exactly two tasks and one repetition')
        if args.capacity_scope == 'selected':
            capacity_entries.extend(dict(selected_entries[0], name='pressure-reserve-' + str(i), files=[])
                                    for i in range(2))
        elif len(capacity_entries) < 4:
            raise ValueError('full capacity must cover the four pressure jobs')
    job_limit = getattr(args, 'capacity_jobs', None)
    if job_limit is not None:
        if (args.capacity_scope != 'all' or type(job_limit) is not int
                or not 1 <= job_limit <= len(capacity_entries) * args.repetitions):
            raise ValueError('explicit retained job capacity requires the full planning set')
        required_jobs = len(selected) * args.repetitions + (2 if args.resource_pressure else 0)
        if required_jobs > job_limit:
            raise ValueError('selected cohort exceeds retained job capacity')
    return selected, oracle_mode, selected_entries, capacity_entries


def exercise(args):
    bounded_policy = acceptance_build_policy(getattr(args, "bounded_policy", None))
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise RuntimeError('exclusive root Linux/KVM required')
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain']).strip():
        raise RuntimeError('clean exact source commit required')
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    lock_path = getattr(args, 'lock', None) or checkout / 'benchmarks/terminal-bench-2.0.lock.json'
    lock = verify(json.loads(lock_path.read_text()), args.registry, args.source)
    if lock['dataset'] == 'terminal-bench@2.1' and (args.oracle_mode != 'original' or args.source_kind == PRUNED_SOURCE):
        raise ValueError('2.0 local repairs cannot be applied to 2.1')
    runtimes = check_runtime()
    check_tasks(lock, args.source)
    selected, oracle_mode, selected_entries, capacity_entries = select_cohort(args, lock['tasks'])
    preparing_ci = getattr(args, 'prepare_ci_fixtures', None) is not None
    preparation_timeout = ci_preparation_timeout(args)
    if preparing_ci:
        from benchmarks.ci_fixtures import ci_task_names
        if (selected != set(ci_task_names(lock)) or args.all
                or args.source_kind != 'dockerfile' or args.oracle_mode != 'original'
                or args.repetitions != 1 or args.resource_pressure
                or args.ci_fixtures is not None or not args.ci_build_inputs or not args.ci_uv):
            raise ValueError('explicit two-task one-time CI preparation inputs required')
    elif getattr(args, 'ci_build_inputs', None) or getattr(args, 'ci_uv', None):
        raise ValueError('CI build inputs are only for one-time fixture preparation')
    ci_fixtures = None
    fixture_sha256 = None
    if getattr(args, 'ci_fixtures', None) is not None:
        from benchmarks.ci_fixtures import load_ci_fixtures
        fixture_sha256 = sha256_file(args.ci_fixtures)
        ci_fixtures = load_ci_fixtures(args.ci_fixtures, fixture_sha256, lock, sorted(selected))
        if lock['dataset'] != 'terminal-bench@2.0' or args.resource_pressure:
            raise ValueError('prepared CI is separate from original full-suite or pressure acceptance')
        if args.proxy_binary is not None or args.temporarily_forward:
            raise ValueError('prepared CI neither enables an outbound proxy nor global forwarding')
        for fixture in ci_fixtures.values():
            if fixture.kernel_sha256 != sha256_file(args.kernel):
                raise ValueError('prepared fixture kernel differs from the approved host kernel')
    disk_ceiling = getattr(args, 'prepared_ci_disk_budget_bytes', None)
    if ci_fixtures is None and disk_ceiling is not None:
        raise ValueError('prepared CI disk ceiling is not a template-build acceptance budget')
    prepared_profile = (prepared_ci_physical_profile(bounded_policy,
        kernel_bytes=args.kernel.stat().st_size, runtime_bytes=args.firecracker.stat().st_size,
        disk_budget_bytes=disk_ceiling) if ci_fixtures is not None else None)
    capacity_basis = {'scope': args.capacity_scope, 'selected_tasks': sorted(selected),
                      'planning_tasks': [t['name'] for t in capacity_entries],
                      'repetitions': args.repetitions}
    if preparing_ci:
        capacity_basis['ci_preparation_timeout_secs'] = preparation_timeout
    job_limit = getattr(args, 'capacity_jobs', None)
    if job_limit is not None:
        capacity_basis['retained_job_limit'] = job_limit
    capacity = plan(capacity_entries, args.repetitions,
                    args.rootfs.stat().st_size, args.kernel.stat().st_size,
                    artifact_bytes=args.artifact_limits['max_bytes'],
                    input_bytes=args.build_input_max_bytes, expanded_bytes=args.build_expanded_max_bytes,
                    job_limit=job_limit)
    deployment = budgets(capacity, args.rootfs.stat().st_size, args.kernel.stat().st_size,
                         profile=load(args.budget_profile), build_required=ci_fixtures is None)
    deployment = acceptance_disk_budgets(deployment, bounded_policy)
    if prepared_profile is not None:
        from tools.template_physical_storage import envelope
        deployment['host']['minimum_free_disk_bytes'] = 2 * envelope(prepared_profile)[0]
        deployment['prepared_ci_disk_budget_bytes'] = disk_ceiling
        deployment['builder_dispatched'] = False
    proxy_binary = getattr(args, 'proxy_binary', None)
    proxy_digest = getattr(args, 'proxy_sha256', None)
    if (proxy_binary is None) != (proxy_digest is None):
        raise ValueError('proxy binary and approved digest must be provided together')
    if proxy_binary is not None:
        # The optional transport has an independent enforced 256 MiB maximum.
        deployment['host']['minimum_available_memory_bytes'] += 256 * MIB
        deployment['host']['minimum_cpus'] += 1
    if args.resource_pressure:
        from template_resource_pressure import validate_cohort
        validate_cohort([t['requirements'] for t in selected_entries], deployment)
    # Trusted operator-provided ext4 base; read metadata without mounting it.
    superblock = subprocess.check_output(['/usr/sbin/dumpe2fs', '-h', str(args.rootfs)],
                                        text=True, env={**os.environ, 'LC_ALL': 'C'}, stderr=subprocess.DEVNULL)
    fields = dict(line.split(':', 1) for line in superblock.splitlines() if ':' in line)
    if ci_fixtures is None and int(fields['Free blocks']) * int(fields['Block size']) < capacity['worker_required_free_bytes']:
        raise RuntimeError('builder base lacks free space for bounded import and candidate')
    stride = capacity['uid_stride']
    if args.uid_base < 1000 or args.uid_base + 2 * stride >= 2**32 - 1:
        raise RuntimeError('invalid exclusive UID ranges')
    if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
        raise RuntimeError('exclusive UID pools occupied')
    if ci_fixtures is None and Path('/proc/sys/net/ipv4/ip_forward').read_text().strip() != '1':
        raise RuntimeError('approved public forwarding required')
    group_parents = (args.cgroup_parent, args.build_cgroup_parent, args.service_cgroup_parent)
    if any(a.is_relative_to(b) for i, a in enumerate(group_parents) for j, b in enumerate(group_parents) if i != j):
        raise RuntimeError('service and VM cgroup parents must be disjoint')
    for parent in (*group_parents, args.scratch_parent):
        if parent.stat().st_uid != 0 or parent.stat().st_mode & 0o077:
            raise RuntimeError('private root parents required')
    for parent in group_parents:
        if (parent / 'cgroup.procs').read_text().strip() or any(p.is_dir() for p in parent.iterdir()):
            raise RuntimeError('exclusive empty delegated cgroups required')
    available = int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines()
                         if line.startswith('MemAvailable:'))) * 1024
    check_headroom(deployment, cpus=len(os.sched_getaffinity(0)), available_memory=available,
                   free_disk=shutil.disk_usage(args.scratch_parent).free)
    build_tools = BUILD_TOOLS | (IGZIP_HASHES if args.igzip else {})
    for name, digest in build_tools.items():
        if sha256_file(args.tools / name) != digest:
            raise RuntimeError('fixed tool mismatch: ' + name)
    require_bounded_host_tools()
    root, namespaces = bounded_acceptance_layout(bounded_policy,
        run_root=getattr(args, 'run_root', None), namespace=args.namespace,
        build_namespace=args.build_namespace)
    if root.parent != args.scratch_parent.resolve(strict=True):
        raise ValueError('bounded run root must be directly inside the approved scratch parent')
    validate_acceptance_socket_paths(root, namespaces, args.firecracker.resolve(strict=True),
)
    root.mkdir(mode=0o700)
    provision_acceptance_coordination(bounded_policy, run_root=root,
        namespace=namespaces[0], build_namespace=namespaces[1])
    print(f'root={root} commit={commit}', flush=True)
    original = [sha256_file(p) for p in (args.kernel, args.rootfs)]
    if (any(not re.fullmatch('[0-9a-f]{16}', n) for n in namespaces)
            or namespaces[0][:6] == namespaces[1][:6]):
        raise ValueError('disjoint deployment namespaces required')
    validate_acceptance_socket_paths(root, namespaces, args.firecracker.resolve(strict=True),
)
    (root / 'capacity.json').write_text(json.dumps(capacity, indent=2))
    (root / 'capacity-basis.json').write_text(json.dumps(capacity_basis, indent=2))
    (root / 'acceptance-budgets.json').write_text(json.dumps(deployment, indent=2))
    public_address = '127.0.0.1:' + str(free_loopback_port())
    key = secrets.token_hex(32)
    firecracker = args.firecracker.resolve(strict=True)
    settings = []
    for i in range(1 if ci_fixtures is not None else 2):
        pool = root / str(i)
        pool.mkdir(mode=0o700)
        for name in ('db', 'state', 'jails'):
            (pool / name).mkdir(mode=0o700)
        cpu = deployment['builder_shape']['cpu_count'] if i else 1
        memory = deployment['builder_shape']['memory_mib'] if i else 2048
        base = descriptor('builder-base' if i else 'runtime-base', args.kernel, args.rootfs, cpu, memory)
        cfg = {'namespace': namespaces[i], 'database': str(pool / 'db/controller.db'),
            'node_state_dir': str(pool / 'state'), 'chroot_base': str(pool / 'jails'),
            'jailer': str(args.jailer), 'firecracker': str(firecracker),
            'kernel': str(args.kernel), 'rootfs': str(args.rootfs), 'uid_base': args.uid_base + stride * i,
            'vcpu_count': cpu, 'memory_mib': memory,
            'host_resources': {'cgroup_parent': str(args.build_cgroup_parent if i else args.cgroup_parent), 'memory_max_bytes': (memory + 256) * MIB},
            'file_policy_digest': 1,
            'public_ipv4_enabled': ci_fixtures is None, **deployment['builder' if i else 'runtime'],
            'templates': {'templates': [base], 'aliases': {'base': base['template_id']},
                'compute_limits': deployment['builder_compute' if i else 'runtime_compute']}}
        if i:
            base['owner'] = None  # Private builder selection remains center authority.
        elif ci_fixtures is not None:
            cfg['templates']['templates'] = [f.template_descriptor() for f in ci_fixtures.values()]
            cfg = prepared_ci_runtime_configuration(cfg, bounded_policy, prepared_profile)
        settings.append(cfg)
    (root / 'builds').mkdir(mode=0o700)
    build = {'root': str(root / 'builds'),
        'registry':{'root':str(root/'registry'),**deployment['registry']},
        'registry_owner_uid':bounded_policy['runtime']['bounded_host']['physical_storage']['public_build']['registry_owner_uid']
            if ci_fixtures is None else args.center_uid,
        'tools': {n: {'path': str(args.tools / n), 'sha256': d} for n, d in build_tools.items()},
        'dns_servers': ['1.1.1.1', '8.8.8.8'],
        'limits': deployment['limits']}
    original_workspace = build['limits']['workspace_bytes']
    build_mode = ('windowed' if ci_fixtures is not None else
                  apply_acceptance_build_policy(settings[0], settings[1], build, bounded_policy))
    deployment['build_mode'] = build_mode
    extra = 0 if ci_fixtures is not None else build['limits']['workspace_bytes'] - original_workspace
    deployment['host']['minimum_free_disk_bytes'] += extra
    if shutil.disk_usage(root).free < deployment['host']['minimum_free_disk_bytes']:
        raise ValueError('bounded build policy exceeds acceptance disk headroom')
    if args.docker_hub_mirror is not None:
        build['docker_hub_mirror'] = args.docker_hub_mirror
    service_record = Path(args.service_cgroup_record)
    if service_record != root / 'service-cgroup.json':
        raise ValueError('service cgroup receipt must belong to this exact acceptance run')
    private_builds = None
    if ci_fixtures is None:
        windows = bounded_policy['builder']['build_retry_windows']
        private_builds = dict(windows=bounded_policy['public']['bounded_builds']['windows'],
            window_duration_ms=windows['window_duration_ms'], max_anchors=windows['max_anchors'],
            builder_revision=settings[1]['templates']['templates'][0]['template_id']+'-r1',
            policy={name:settings[1][name] for name in ('public_ipv4_enabled','max_timeout_secs',
                'max_lifetime_secs','startup_timeout_secs')})
    registry = {'root': str(root / 'registry'), **deployment['registry']}
    services = RuntimeServices.prepare(args,root,settings,[config['templates'] for config in settings],
        key,public_address,record=service_record,job_count=capacity['jobs'],
        private_builds=private_builds,registry_config=registry,activation_ceiling_bytes=disk_ceiling)
    settings = services.native_pools  # Exactly the profile Rust initializes.
    registry_root = services.layout['registry']
    urls = [services.sdk_url] * 3  # Every public SDK/build request goes to center.
    binary = services.processes.bindings['host-0']['binary']
    build_path = root / 'build-control/build.json'
    if ci_fixtures is None:
        build.update(bind=services.layout['helper_backend_address'],
            runtime_config=str(services.processes.bindings['host-0']['config']),
            builder_config=str(services.processes.bindings['host-1']['config']),
            registry=services.center_config['template_registry'], registry_owner_uid=args.center_uid,
            builder_template=services.center_config['templates']['templates'][-1],
            center=dict(private_address=services.layout['private_address'],domain='example.invalid',
                credential_file=str(services.layout['build_key_file']),
                private_windows={key:private_builds[key] for key in ('window_duration_ms','max_anchors')},
                max_timeout_secs=settings[1]['max_timeout_secs'],max_lifetime_secs=settings[1]['max_lifetime_secs']))
        build_path.write_text(json.dumps(build))
        build_path.chmod(0o600)
        validate_build_service(build_path,require_physical_lease=False)
    from tools.template_physical_storage import envelope
    activation_bytes=2*envelope(settings[0]['bounded_host']['physical_storage'])[0]
    if ci_fixtures is not None and activation_bytes>disk_ceiling:
        raise ValueError('dual-service/cache activation exceeds approved prepared CI ceiling')
    deployment['host']['minimum_free_disk_bytes']=max(deployment['host']['minimum_free_disk_bytes'],activation_bytes)
    (root/'acceptance-budgets.json').write_text(json.dumps(deployment,indent=2))
    supervisor, log = None, None
    transport = (BenchmarkProxy(root / 'proxy', namespaces, proxy_binary, proxy_digest,
                               port=getattr(args, 'proxy_socks_port', 7897))
                 if proxy_binary is not None else None)
    service_prefix = service_cgroup.prefix(service_record)
    service_samples = []
    def observe_services(*, require_builder=True, minimum_relays=0):
        services.processes.observe()
        processes = list(services.processes.children.values())
        if require_builder and supervisor is not None:
            process_facts(supervisor,Path(sys.executable),build_path,service_record,uid=0,gid=0,
                argv=[sys.executable,str(checkout/'tools/template_build_service.py'),'--config',str(build_path)])
            processes.append(supervisor)
        if any(p.poll() is not None for p in processes):
            raise RuntimeError('service exited; retain original resources for recovery')
        return observe_service_resources(binary, service_record, [p.pid for p in processes],
            service_samples, root / 'service-resources.json',
            minimum_relays=minimum_relays)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    rows, complete, all_passed = [], False, True
    pressure_report = ({'status': 'not_run', 'reason': 'requires_two_published_audited_templates'}
                       if args.resource_pressure else None)
    create_identities = {}
    physical_identities = []
    def audit_shape(env):
        observe_services()
        template = env.template
        # Planned offline audit, not a guessed GLOBAL suffix or a public host.
        center_audit=services.planned_audit('center')
        create_identities[env.sandbox_id]=find_runtime_create(center_audit,env.runtime_intent['metadata'],
            deployment=services.center_config['deployment_id'])
        services.processes.stop('host-0')
        try:
            native_id=services.native_id_for(env.sandbox_id)
            secret,name,claim=record_for(root/'0',native_id=native_id)
            network=json.loads((root/'0/jails/.claims'/name/'network.json').read_text())
            if network['resource_id']!=name:
                raise RuntimeError('original network resource differs from mapped VM')
            physical=claim|dict(network_inode=network['namespace_inode'])
            physical_identities.append(physical)
        finally:
            services.processes.start('host-0')
        services.wait_ready(for_admission=False)
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
                'native_id':native_id,'create_identity':create_identities[env.sandbox_id],
                'physical_identity':physical,
                'pid': claim['pid'], 'start_time_ticks': claim['start_time_ticks'],
                'disk_inode': disk.stat().st_ino, 'disk_logical_bytes': disk.stat().st_size,
                'disk_allocated_bytes': disk.stat().st_blocks * 512,
                'template_allocated_bytes': source.stat().st_blocks * 512, 'machine': machine}
    def completion_audit(row):
        # A lost reply may precede completion of the same admitted disk copy.
        # Observe through its original deadline plus a bounded cleanup margin.
        original_deadline = (row.get('build') or {}).get('deadline', time.time())
        # Publication can admit its own check VM after a slow artifact check.
        # Keep both controllers alive through that VM's bounded preparation
        # and lifetime, rather than killing the reclaimer after a 90s pause.
        deadline = (time.monotonic() + max(0, original_deadline - time.time())
                    + 2 * capacity['max_build_seconds'] + 90)
        result = {}
        if row.get('create_dispatched'):
            if not row.get('runtime_intent'):
                raise RuntimeError('missing runtime creation intent; retain original identity')
            expected_node=services.host_configs[0]['inbox']['node']
            def read_host(node):
                if node!=expected_node:
                    raise RuntimeError('runtime recovery selected a foreign registered host')
                return services.planned_audit('host-0')
            def delete(ident):
                services.wait_ready(for_admission=False)
                row['kill_dispatched']=True  # Survives later completion callbacks.
                return fetch(opener,urls[0],'DELETE','/sandboxes/'+ident,key)[0]
            recovered=recover_runtime_create(row['runtime_intent'],
                read_center=lambda:services.planned_audit('center'),read_host=read_host,
                deployment=services.center_config['deployment_id'],owner=1,delete=delete,
                deadline=deadline,sandbox_id=row.get('sandbox_id'),
                create_identity=create_identities.get(row.get('sandbox_id')),
                delete_dispatched=bool(row.get('kill_dispatched')),
                prepared_ci=ci_fixtures is not None)
            result.update(sandbox_id=recovered['sandbox_id'],runtime_recovery=recovered)
        while True:
            pending = pending_build_reservations(root / 'builds', prepared=ci_fixtures is not None, row=row)
            if not pending:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError('original build still owns its reservation')
            time.sleep(0.5)
        for i in range(len(settings)):
            pool = root / str(i)
            while True:
                if services.processes.children['host-'+str(i)].poll() is not None:
                    raise RuntimeError('private host exited before exact cleanup')
                with sqlite3.connect(pool / 'db/controller.db') as db:
                    phases = [json.loads(r[0])['phase'] for r in db.execute('SELECT state_blob FROM sandboxes')]
                    pending_actions = db.execute("SELECT COUNT(*) FROM outbox WHERE dispatch_state='dispatched'").fetchone()[0]
                try:
                    assert not pending_actions and all(p == 'Destroyed' for p in phases)
                    claims = pool / 'jails/.claims'
                    if claims.exists():
                        check_no_resources(pool, namespaces[i], firecracker, args.uid_base + i * stride,
                                           physical_identities if i==0 else (),native=settings[i])
                        for claim in claims.iterdir():
                            audit_gone(pool, firecracker, args.build_cgroup_parent if i else args.cgroup_parent, claim.name)
                    if i==0:
                        for identity in physical_identities:
                            audit_gone(pool,firecracker,args.cgroup_parent,identity['resource_id'],identity=identity)
                    break
                except AssertionError:
                    if time.monotonic() >= deadline:
                        raise RuntimeError('pool still owns an unreclaimed identity') from None
                    time.sleep(.5)
        if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
            raise RuntimeError('assigned UID still owns a live process')
        result['exact_cleanup'] = 'passed'
        if row.get('build_dispatched'):
            from tools.template_build_client import BuildClient
            client = BuildClient(urls[2], key)
            # A long task may outlive a bounded build's retained query record.
            # Preserve the terminal result already observed before running it;
            # an uncertain result still requires GET, never a replacement POST.
            result.update(completion_build_observation(row,
                read_build=lambda job: client.request('GET', '/sandproof/template-builds/' + job)))
        template = row.get('template') or (result.get('build') or {}).get('template')
        if template:
            source = (ci_fixtures[row['name']].rootfs if ci_fixtures is not None else
                      registry_root / 'artifacts' / template['templateId'].removeprefix('tpl-') / 'rootfs.ext4')
            assert sha256_file(source) == template['rootfsSha256']
        return result

    service_report = None
    try:
        if transport is not None:
            transport.start()
        service_cgroup.prepare(args.service_cgroup_parent, 'svc-' + namespaces[0], service_record,
                               **deployment['service_cgroup'])
        services.start()
        services.wait_ready()
        supervisor, log = launch_build_service(checkout, root, service_prefix,
                                               prepared=ci_fixtures is not None)
        if supervisor is not None:
            for _ in range(60):
                try:
                    # This known route reaches the private helper only AFTER
                    # center authentication. A generic center 404 proves none
                    # of the builder boundary and must not signal readiness.
                    if fetch(opener,urls[2],'GET','/sandproof/template-builds/'+'0'*32,key)[0]==404:
                        break
                except OSError:
                    if supervisor.poll() is not None:
                        raise RuntimeError('build supervisor startup failed')
                    time.sleep(0.5)
            else:
                raise RuntimeError('build endpoint did not become ready')
        observe_services()
        deployment['host']['control_allowance_enforced_by_cgroup'] = True
        (root / 'acceptance-budgets.json').write_text(json.dumps(deployment, indent=2))
        helper=(dict(config=build_path,process=supervisor,binary=Path(sys.executable),
            argv=[sys.executable,str(checkout/'tools/template_build_service.py'),'--config',str(build_path)])
            if supervisor is not None else None)
        execution,_=services.execution(args.run_identity,root/'acceptance-budgets.json',helper=helper)
        write_once(root/'execution-manifest.json',execution)
        for repetition in range(args.repetitions):
            output = root / f'run-{repetition}'
            output.mkdir(mode=0o700)
            (output / 'inputs.json').write_text(json.dumps({'source': lock['source'], 'runtimes': runtimes,
                'capacity_basis': capacity_basis,
                'oracle_mode': oracle_mode,
                'original_environment': args.source_kind != PRUNED_SOURCE and ci_fixtures is None and not preparing_ci,
                'source_kind': args.source_kind, 'artifact_limits': args.artifact_limits,
                'failure_diagnostics': args.failure_diagnostics,
                'network_transport': network_transport_evidence(transport, prepared=ci_fixtures is not None),
                'task_files': {t['name']: t['files_sha256'] for t in lock['tasks']}}, indent=2))
            run_args = SimpleNamespace(source=args.source, output=output, task=sorted(selected), all=args.all,
                oracle_mode=oracle_mode,
                api_url=urls[0], build_url=urls[2], build_mode=build_mode, source_kind=args.source_kind,
                failure_diagnostics=args.failure_diagnostics,
                artifact_limits=args.artifact_limits, start_audit=audit_shape, completion_audit=completion_audit)
            if preparing_ci:
                run_args.prepare_ci_fixtures = args.prepare_ci_fixtures
                run_args.preparation_timeout_secs = preparation_timeout
                run_args.ci_build_inputs, run_args.ci_uv = args.ci_build_inputs, args.ci_uv
                run_args.ready_artifacts_root, run_args.kernel = registry_root / 'artifacts', args.kernel
            if ci_fixtures is not None:
                from benchmarks.ci_verifier import prepare_ci_verifier_task
                from harbor.models.task.task import Task
                run_args.ci_fixtures = ci_fixtures
                run_args.verifier_task_factory = lambda task, entry, destination: Task(
                    prepare_ci_verifier_task(task, entry, destination))
            old_key = os.environ.get('SANDPROOF_TEST_API_KEY')
            os.environ['SANDPROOF_TEST_API_KEY'] = key
            with (root / f'run-{repetition}.log').open('wb') as out:
                import io
                text_out = io.TextIOWrapper(out, write_through=True)
                try:
                    with contextlib.redirect_stdout(text_out), contextlib.redirect_stderr(text_out):
                        if preparing_ci:
                            from benchmarks.run_ci_prepare import prepare_selected
                            asyncio.run(prepare_selected(run_args, lock))
                        else:
                            asyncio.run(run_selected(run_args, lock))
                finally:
                    text_out.detach()
                    if old_key is None:
                        os.environ.pop('SANDPROOF_TEST_API_KEY', None)
                    else:
                        os.environ['SANDPROOF_TEST_API_KEY'] = old_key
            report = json.loads((output / 'full-set.json').read_text())
            current = [r for r in report['tasks'] if r['status'] != 'not_run']
            if args.resource_pressure and ready_for_pressure(current):
                from template_resource_pressure import exercise as pressure
                def restart_builds():
                    nonlocal supervisor
                    supervisor = subprocess.Popen([*service_prefix, sys.executable, str(checkout / 'tools/template_build_service.py'),
                        '--config', str(build_path)], stdout=log, stderr=subprocess.STDOUT)
                    return supervisor
                try:
                    pressure_report = asyncio.run(pressure(root=root, services=services, key=key, build_config=build_path,
                        rows=current, deployment=deployment, supervisor=supervisor, restart=restart_builds,
                        source_entry=selected_entries[0], audit_shape=audit_shape, completion_audit=completion_audit,
                        build_mode=build_mode,
                        service_audit=lambda count: observe_services(require_builder=False, minimum_relays=count)))
                except Exception:
                    # Retain the separate failed probe and original task scores.
                    # The common physical audit below must still pass before exit.
                    pressure_report = json.loads((root / 'resource-pressure.json').read_text())
                    if pressure_report['status'] in ('running', 'passed'):
                        raise
                (root / 'resource-pressure.json').write_text(json.dumps(pressure_report, indent=2) + '\n')
            # Exact reclaim, not just the SDK's kill boolean.
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline:
                pending = pending_build_reservations(root / 'builds', prepared=ci_fixtures is not None)
                if pending == 0:
                    break
                time.sleep(0.5)
            if pending:
                raise RuntimeError('original build identity still owns cleanup budget')
            for i in range(len(settings)):
                with sqlite3.connect(root / str(i) / 'db/controller.db') as db:
                    phases = [json.loads(r[0])['phase'] for r in db.execute('SELECT state_blob FROM sandboxes')]
                if any(p != 'Destroyed' for p in phases):
                    raise RuntimeError('durable terminal cleanup incomplete')
                claims = root / str(i) / 'jails/.claims'
                if claims.exists():
                    check_no_resources(root / str(i), namespaces[i], firecracker, args.uid_base + i * stride,
                                       physical_identities if i==0 else (),native=settings[i])
                    for claim in claims.iterdir():
                        audit_gone(root / str(i), firecracker, args.build_cgroup_parent if i else args.cgroup_parent, claim.name)
                else:
                    for parent in (root / str(i) / 'jails', Path('/run/netns'), Path('/sys/fs/cgroup')):
                        assert not list(parent.rglob(f'vm-{namespaces[i]}-*'))
                if i==0:
                    for identity in physical_identities:
                        audit_gone(root/'0',firecracker,args.cgroup_parent,identity['resource_id'],identity=identity)
            if processes_in_uid_range(args.uid_base, args.uid_base + 2 * stride - 1):
                raise RuntimeError('exact pool UID process remains')
            for row in current:
                row['repetition'] = repetition
                template = row.get('template') or (row.get('build') or {}).get('template')
                if template:
                    source = (ci_fixtures[row['name']].rootfs if ci_fixtures is not None else
                              registry_root / 'artifacts' / template['templateId'].removeprefix('tpl-') / 'rootfs.ext4')
                    assert sha256_file(source) == template['rootfsSha256']
            rows.extend(current)
            observe_services()
            report['exact_cleanup_audit'] = ('passed' if all(r.get('exact_cleanup') == 'passed' for r in current)
                                             else 'unknown')
            report['capacity_basis'] = capacity_basis
            (output / 'full-set.json').write_text(json.dumps(report, indent=2) + '\n')
            outcome = repetition_outcome(current, selected, oracle_mode=oracle_mode,
                resource_pressure=args.resource_pressure, pressure_report=pressure_report)
            if ci_fixtures is not None:
                outcome.update(original_verifier=False, original_oracle_verifier=False,
                               original_oracle=True, original_test_assertions=True,
                               fixture_sha256=fixture_sha256)
            if preparing_ci:
                outcome.update(execution_passed=False, original_verifier=False,
                               original_oracle_verifier=False, preparation_only=True,
                               fixture_preparation_passed=(len(current) == len(selected)
                                   and all(r['status'] == 'prepared' and r.get('exact_cleanup') == 'passed'
                                           for r in current)))
            execution,processes=services.execution(args.run_identity,root/'acceptance-budgets.json',
                helper=(dict(config=build_path,process=supervisor,binary=Path(sys.executable),
                    argv=[sys.executable,str(checkout/'tools/template_build_service.py'),'--config',str(build_path)])
                    if supervisor is not None else None))
            if execution!=json_file(root/'execution-manifest.json',limit=32768):
                raise RuntimeError('original execution configuration/material receipt changed')
            service_report = {'commit': commit,
                'services':execution['services'],'execution_manifest':execution,'service_processes':processes,
                'service_boundary':dict(public_api='center',center_unprivileged=True,
                    redis_authority='validated',host_transport='mutual-tls',host_public_management=False),
                'kernel_sha256': original[0], 'builder_base_sha256': original[1], 'tools': build_tools,
                'source_kind': args.source_kind, 'tasks': rows, 'fixed_task_count': lock['task_count'],
                'dataset': lock['dataset'], 'task_source': lock['source'],
                'capacity_basis': capacity_basis,
                'original_environment': args.source_kind != PRUNED_SOURCE and ci_fixtures is None and not preparing_ci,
                'docker_hub_mirror': args.docker_hub_mirror,
                'network_transport': network_transport_evidence(transport, prepared=ci_fixtures is not None),
                'failure_diagnostics': args.failure_diagnostics,
                'acceptance_budgets': deployment, 'resource_pressure': pressure_report,
                'prepared_ci_physical': ({'disk_budget_bytes': disk_ceiling,
                    'activation_bytes': activation_bytes,
                    'pool_roles': ['runtime'], 'builder_dispatched': False, 'public_build': False}
                    if ci_fixtures is not None else None),
                'service_cgroup': service_samples[-1],
                'service_storage': services.storage_peaks(),
                'no_process_network_cgroup_jail_residue': True,
                **outcome, 'paid_model_used': False}
            (root / 'report.json').write_text(json.dumps(service_report, indent=2) + '\n')
            passed = outcome.get('fixture_preparation_passed', outcome['execution_passed'])
            all_passed = all_passed and passed
            print(json.dumps({'repetition': repetition, 'execution_passed': outcome['execution_passed'],
                              **({'fixture_preparation_passed': passed} if preparing_ci else {}),
                              'tasks': [{k: r.get(k) for k in ('name', 'status', 'stage', 'error_type', 'exact_cleanup')} for r in current]}), flush=True)
            if not passed:
                print('task failures retained; no pass claim for this batch', flush=True)
            if any(r.get('exact_cleanup') != 'passed' for r in current):
                raise RuntimeError('unresolved per-task identity; no further repetition permitted')
        assert original == [sha256_file(p) for p in (args.kernel, args.rootfs)]
        complete = True
        print('generic task run and cleanup complete; report=' + str(root / 'report.json'), flush=True)
        return 0 if all_passed else 1
    finally:
        stop_errors = []
        for p in [supervisor]:
            try:
                stop_child(p)
            except Exception as error:
                stop_errors.append(type(error).__name__)
        if log:
            log.close()
        try:
            services.stop_all()
            if service_report is not None:
                service_report['service_storage'] = services.storage_peaks()
                (root / 'report.json').write_text(json.dumps(service_report, indent=2) + '\n')
            if complete:
                services.cleanup_staging()
        except Exception as error:
            stop_errors.append(type(error).__name__)
        if service_record.exists():
            try:
                # Keep the original group usable for service restart while any
                # VM identity or cleanup remains unresolved outside this leaf.
                snapshot = (service_cgroup.remove(service_record) if complete else
                            service_cgroup.observe(service_record) | {'removed': False,
                                'reason': 'retained_for_original_vm_recovery'})
                (root / 'service-cleanup.json').write_text(json.dumps(snapshot, indent=2))
            except Exception as error:
                stop_errors.append(type(error).__name__)
        # Preserve private credentials until the operator independently audits
        # both pools, including failures; deleting them can prevent recovery.
        if not complete:
            print('failure evidence retained at ' + str(root), flush=True)
        if transport is not None and transport.state['phase'] not in ('new', 'removed'):
            try:
                evidence = transport.finish(quiescent=complete)
                (root / 'proxy-cleanup.json').write_text(json.dumps(evidence, indent=2))
                if evidence['phase'] != 'removed':
                    stop_errors.append('ProxyRetainedForOriginalVmRecovery')
            except Exception as error:
                stop_errors.append(type(error).__name__)
        if stop_errors:
            (root / 'service-cleanup-errors.json').write_text(json.dumps(stop_errors))
            raise RuntimeError('service cleanup incomplete; retain recorded resources')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument('--run-identity',required=True,help='approved supervisor run SHA256')
    for name in ('firecracker', 'jailer', 'kernel', 'rootfs', 'tools', 'scratch-parent',
                 'cgroup-parent', 'build-cgroup-parent', 'service-cgroup-parent', 'registry', 'source'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    parser.add_argument('--lock', type=Path, help='explicit independently frozen task version; legacy 2.0 default retained')
    parser.add_argument("--bounded-policy", type=Path, required=True,
                        help="explicit complete policies for both pools and public build storage")
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument('--task', action='append')
    selection.add_argument('--all', action='store_true')
    parser.add_argument('--capacity-scope', choices=('selected', 'all'), default='selected',
                        help='plan quotas for selected tasks or the full locked set, without changing execution selection')
    parser.add_argument('--capacity-jobs', type=int,
                        help='explicit retained build jobs per cohort with --capacity-scope all; includes repetitions and pressure jobs')
    parser.add_argument('--namespace')
    parser.add_argument('--build-namespace')
    parser.add_argument('--run-root', type=Path,
                        help='exact fresh run root already declared in the full physical profile')
    parser.add_argument('--source-kind', choices=(*SOURCE_KINDS, 'prepared-ci'), default='oci')
    parser.add_argument('--ci-fixtures', type=Path,
                        help='operator-pinned prepared CI manifest; no runtime Internet or original-environment claim')
    parser.add_argument('--prepared-ci-disk-budget-bytes', type=int,
                        help='required prepared-CI total activation ceiling, including twice the physical envelope; at most 200 GiB')
    parser.add_argument('--prepare-ci-fixtures', type=Path,
                        help='one-time fresh private destination; builds clean fixtures, never scores tasks')
    parser.add_argument('--ci-preparation-timeout', type=int,
                        help='one-time fixture build budget: 1..900 seconds, default 900; invalid in scoring mode')
    parser.add_argument('--ci-build-inputs', type=Path,
                        help='operator-reviewed map of the two original task names to immutable OCI digests')
    parser.add_argument('--ci-uv', type=Path, help='approved host uv binary, copied only into the guest builder context')
    parser.add_argument('--oracle-mode', choices=('original', 'repaired'), default='original')
    parser.add_argument('--igzip', action='store_true', help='use the fixed optional ISA-L decoder pair in the build VM')
    parser.add_argument('--failure-diagnostics', action='store_true',
                        help='retain registered post-score guest probes without changing original scores')
    parser.add_argument('--repetitions', type=int, choices=(1, 2), default=1)
    parser.add_argument('--resource-pressure', action='store_true',
                        help='test two Ready templates after exact task cleanup; original failed scores remain failed')
    parser.add_argument('--budget-profile', type=Path, default=PROFILE,
                        help='explicit operator acceptance profile; original larger profile remains the default')
    parser.add_argument('--docker-hub-mirror',
                        help='trusted HTTPS registry hostname for OCI transport only; original task references remain unchanged')
    parser.add_argument('--temporarily-forward', action='store_true')
    parser.add_argument('--proxy-binary', type=Path,
                        help='approved root-owned Mihomo; optional per-run TUN to existing loopback SOCKS proxy')
    parser.add_argument('--proxy-sha256', help='required with --proxy-binary; no automatic download or version change')
    parser.add_argument('--proxy-socks-port', type=int, default=7897, help='host loopback only; never exposed to guests')
    parser.add_argument('--build-input-max-bytes', type=int, default=INPUT_BYTES,
                        help='bounded build input/download bytes; default 4 GiB, service maximum 16 GiB')
    parser.add_argument('--build-expanded-max-bytes', type=int, default=EXPANDED_BYTES,
                        help='bounded expanded image layers; default 8 GiB, service maximum 32 GiB')
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
