"""Small, opt-in KVM acceptance of the complete single-runtime bounded mode.

The operator supplies reviewed fixed paths and budgets; this probe never moves a
physical profile to a random directory, deletes a database, changes host network
settings, or invents a host receipt. Two positions are reused while one original
VM remains alive. A separate reviewed Rust driver checks real late host actions.
Evidence remains private on both success and failure. This is a finite engineering
experiment, not a proof of unbounded service uptime or a two-pool build acceptance.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
from contextlib import closing
import copy
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import sqlite3
import stat
import subprocess
import sys
import threading
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import check_kvm_host as host
from tools.template_physical_storage import envelope
from tools.bounded_acceptance import require_bounded_host_tools


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def private_json(path):
    path = host.trusted_path(Path(path))
    meta = path.stat()
    if not stat.S_ISREG(meta.st_mode) or meta.st_mode & 0o077 or meta.st_size > 256 * 1024:
        raise ValueError('private bounded JSON file required')
    return json.loads(path.read_bytes(), object_pairs_hook=host.unique_keys)


def write_private(path, value):
    # New evidence files only; do not overwrite a prior run's input or report.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())


def node_configuration(settings):
    """Same NodeConfig projection as sandbox-server's production assembly."""
    result = {key: copy.deepcopy(settings[key]) for key in (
        'namespace', 'jailer', 'firecracker', 'chroot_base', 'kernel', 'rootfs',
        'vcpu_count', 'memory_mib', 'file_policy_digest')}
    result['state_dir'] = settings['node_state_dir']
    result['host_resources'] = copy.deepcopy(settings.get('host_resources'))
    bounded = copy.deepcopy(settings['bounded_host'])
    transfers = settings['data_transfer']
    bounded.update(max_data_routes_per_slot=transfers['max_transfers_per_vm'],
                   max_data_routes_global=transfers['max_transfers'],
                   data_route_control_reserve=transfers['control_reserve'])
    result['bounded_host'] = bounded
    return result


def validate_runtime_configuration(settings, rounds, wait_seconds):
    """The same fixed runtime limits for standalone and shared DK acceptance."""
    if type(rounds) is not int or not 4 <= rounds <= 16:
        raise ValueError('use a finite four-to-sixteen-round experiment')
    if type(wait_seconds) is not int or not 5 <= wait_seconds <= 120:
        raise ValueError('bounded maintenance wait required')
    bounded = settings.get('bounded_host', {})
    maintenance = settings.get('maintenance', {})
    capacity = maintenance.get('capacity')
    if (not capacity or bounded.get('console_policy') != 'discard'
            or bounded.get('maintenance', {}).get('max_host_slots') != 2
            or settings.get('max_active_global') != 2
            or settings.get('max_active_per_tenant') != 2
            or {'bind','domain','credentials','request_admission','internal_build_vms',
                'build_retry_windows','template_registry'} & settings.keys()):
        raise ValueError('fresh complete two-position runtime-only configuration required')
    if settings.get('max_timeout_secs', 0) < 600 or settings.get('max_lifetime_secs', 600) < 600:
        raise ValueError('explicit probe lifetime must fit both unchanged deployment limits')
    if ('max_vm_id' in settings or maintenance.get('retention_ms', 0) <= 0
            or maintenance['retention_ms'] + 3 * settings['expiry_scan_seconds'] * 1000 >= wait_seconds * 1000):
        raise ValueError('experiment must cross the old VM bound and observe the unchanged retention period')
    profile = bounded.get('physical_storage')
    peak, inodes = envelope(profile)
    if len(profile['pools']) != 1 or profile['public_build'] is not None:
        raise ValueError('this narrow probe requires exactly its one runtime physical pool')
    pool = profile['pools'][0]
    if (pool['role'] != 'runtime' or pool['namespace'] != settings['namespace']
            or any(pool[key] != settings[key] for key in ('database', 'node_state_dir', 'chroot_base'))
            or pool['max_host_slots'] != 2
            or pool['rootfs_budget_bytes'] != settings['max_disk_bytes_global'] - settings['cleanup_reserve_bytes']
            or pool['cleanup_reserve_bytes'] != settings['cleanup_reserve_bytes']
            or pool['host_metadata_bytes'] + capacity['max_online_storage_bytes'] > settings['cleanup_reserve_bytes']):
        raise ValueError('physical pool does not match the exact accepted paths and quotas')
    node_configuration(settings)  # Reject incomplete route projection before launch.
    return pool, peak, inodes


def validate_configuration(settings, host_config, rounds, wait_seconds):
    pool, peak, inodes = validate_runtime_configuration(settings, rounds, wait_seconds)
    if host_config['chroot_base'] != settings['chroot_base']:
        raise ValueError('preflight must inspect the exact runtime chroot')
    for name in host.ARTIFACTS:
        if host_config['artifacts'][name]['path'] != settings[name]:
            raise ValueError('preflight artifact differs from runtime: ' + name)
    if host_config['min_free_disk_bytes'] < 2 * peak:
        raise ValueError('preflight omits the complete fresh physical activation peak')
    return pool, peak, inodes


def retention_slots(services, rounds):
    """Use only the original protected lane budget; never enlarge it for churn."""
    retained = services.host_configs[0]['inbox']['max_sandboxes']
    registration = services.center_config['nodes'][0]['registration']
    if (type(retained) is not int or retained <= 0
            or type(registration['retention_slots']) is not int
            or retained != registration['retention_slots'] or rounds <= retained):
        raise ValueError('original fixed retention slots must match and churn must exceed their count')
    return retained


def file_usage(database):
    paths = [Path(database), Path(str(database) + '-wal'), Path(str(database) + '-shm')]
    return {name: path.stat().st_size if path.exists() else 0
            for name, path in zip(('database', 'wal', 'shm'), paths)}


class FilePeaks:
    """Sample physical files, never retain a SQLite reader across a write."""
    def __init__(self, database):
        self.database = database
        self.peaks = dict(database=0, wal=0, shm=0)
        self.stopped = threading.Event()
        self.error = None
        self.thread = threading.Thread(target=self.run, daemon=True)

    def sample(self):
        for name, value in file_usage(self.database).items():
            self.peaks[name] = max(self.peaks[name], value)

    def run(self):
        while not self.stopped.is_set():
            try:
                self.sample()
            except FileNotFoundError:
                pass  # A WAL can be removed between exists and stat.
            except Exception as error:
                self.error = type(error).__name__
                return
            self.stopped.wait(0.05)

    def stop(self):
        self.stopped.set()
        self.thread.join(timeout=2)
        self.sample()
        if self.thread.is_alive() or self.error:
            raise AssertionError('physical file sampler incomplete')


def database_snapshot(database):
    # WAL-aware read-only connection. Close promptly: persistent external readers
    # are intentionally not supported by this deployment's write contract.
    with closing(sqlite3.connect(f'file:{database}?mode=ro', uri=True, timeout=2)) as connection:
        tables = [row[0] for row in connection.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
        counts, payload = {}, 0
        for table in tables:
            if not re.fullmatch(r'[a-z_]+', table):
                raise AssertionError('unexpected controller table')
            rows = connection.execute('SELECT * FROM "' + table + '"').fetchall()
            counts[table] = len(rows)
            if table not in ('controller_inputs', 'controller_checkpoint'):
                payload += sum(len(value.encode()) if isinstance(value, str) else len(value)
                               if isinstance(value, bytes) else 8 if value is not None else 0
                               for row in rows for value in row)
        checkpoint = connection.execute(
            'SELECT generation,input_cut,length(payload) FROM controller_checkpoint WHERE id=1').fetchone()
        assert checkpoint is not None, 'complete mode has no checkpoint'
        tail = connection.execute(
            'SELECT count(*),coalesce(sum(length(input_json)),0) FROM controller_inputs WHERE sequence>?',
            (checkpoint[1],)).fetchone()
        phases = {ident: json.loads(blob)['phase'] for ident, blob in connection.execute(
            'SELECT sandbox_id,state_blob FROM sandboxes')}
        contexts = [json.loads(row[0]) for row in connection.execute(
            'SELECT context_json FROM controller_host_slots ORDER BY position')]
        frontier = {(kind, scope): int(value) for kind, scope, value in connection.execute(
            'SELECT kind,scope,high_water FROM controller_id_highwaters')}
        deleted = connection.execute('SELECT deleted_through FROM controller_checkpoint_gc WHERE id=1').fetchone()[0]
    return dict(counts=counts, sql_payload_bytes=payload, phases=phases, contexts=contexts,
                checkpoint=dict(generation=checkpoint[0], input_cut=checkpoint[1], bytes=checkpoint[2],
                                deleted_through=deleted), tail=dict(rows=tail[0], bytes=tail[1]),
                highwaters={kind + ':' + scope: value for (kind, scope), value in frontier.items()},
                files=file_usage(database))


def assert_budget(snapshot, settings):
    policy, cap = settings['maintenance'], settings['maintenance']['capacity']
    for name in ('database', 'wal', 'shm'):
        assert snapshot['files'][name] <= cap['max_' + name + '_bytes'], name + ' physical budget exceeded'
    assert sum(snapshot['files'].values()) <= cap['max_online_storage_bytes']
    assert snapshot['checkpoint']['bytes'] <= policy['max_checkpoint_bytes']
    assert snapshot['tail']['rows'] <= policy['max_tail_inputs']
    assert snapshot['tail']['bytes'] <= policy['max_tail_bytes']
    assert snapshot['sql_payload_bytes'] <= cap['max_sql_bytes']
    actual_rows = sum(count for table, count in snapshot['counts'].items()
                      if table not in ('controller_inputs', 'controller_checkpoint', 'controller_host_slots'))
    assert actual_rows <= cap['max_sql_rows']


def assert_reopen_frontiers(before, after):
    # Empty startup can append the maintenance clock/expiry input before the
    # first HTTP read. That is not a newly allocated sandbox/effect/receipt.
    # Never allow an identity frontier to move, disappear, or be recreated.
    assert set(before) == set(after) and 'input:' in before, 'reopen changed allocator domains'
    for kind, value in before.items():
        if kind == 'input:':
            assert after[kind] >= value, 'reopen reset the input frontier'
        else:
            assert after[kind] == value, 'reopen changed permanent allocator frontier: ' + kind


def context_for(snapshot, local_number):
    if type(local_number) is not int or local_number <= 0:
        raise ValueError('exact original audit local mapping required')
    matches = [value for value in snapshot['contexts'] if value['binding']['sandbox'] == local_number]
    assert len(matches) == 1, 'sandbox has no unique current physical position'
    return matches[0]


def capture_actions(database, ident):
    with closing(sqlite3.connect(f'file:{database}?mode=ro', uri=True, timeout=2)) as connection:
        return [json.loads(row[0]) for row in connection.execute(
            'SELECT action_json FROM outbox WHERE sandbox_id=? ORDER BY id', (ident,))]


def tree_records(roots):
    result = {}
    for root in roots:
        root = Path(root)
        if not root.exists():
            continue
        meta = root.lstat()
        assert stat.S_ISDIR(meta.st_mode), 'owned metadata root changed type'
        result[str(root)] = (meta.st_dev, meta.st_ino, stat.S_IFMT(meta.st_mode), meta.st_size, None)
        for directory, directories, files in os.walk(root, followlinks=False):
            for name in directories + files:
                path = Path(directory) / name
                meta = path.lstat()
                assert not stat.S_ISLNK(meta.st_mode), 'unexpected link in owned metadata'
                result[str(path)] = (meta.st_dev, meta.st_ino, stat.S_IFMT(meta.st_mode), meta.st_size,
                                     digest(path) if stat.S_ISREG(meta.st_mode) else None)
    return result


def metadata_usage(settings):
    roots = (Path(settings['node_state_dir']), Path(settings['chroot_base']) / '.claims')
    records = tree_records(roots)
    allocated = {}
    for name, value in records.items():
        meta = Path(name).lstat()
        assert (meta.st_dev, meta.st_ino) == value[:2], 'metadata changed while measuring'
        allocated[(meta.st_dev, meta.st_ino)] = meta.st_blocks * 512
    return dict(entries=len(records),
                logical_file_bytes=sum(value[3] for value in records.values() if value[2] == stat.S_IFREG),
                allocated_bytes=sum(allocated.values()))


def live_identity(settings, context, ticks):
    name = f"vm-{settings['namespace']}-{context['binding']['vm']}"
    claim = Path(settings['chroot_base']) / '.claims' / name
    process = private_json(claim / 'record.json')
    assert process['resource_id'] == name and ticks(process['pid']) == process['start_time_ticks']
    namespace = Path('/run/netns') / name
    network = private_json(claim / 'network.json')
    assert namespace.stat().st_ino == network['namespace_inode']
    jail = Path(settings['chroot_base']) / Path(settings['firecracker']).name / name
    disk = (jail / 'root/rootfs.ext4').stat()
    group = Path(settings['host_resources']['cgroup_parent']) / name
    cgroup = private_json(claim / 'cgroup.json')
    assert group.stat().st_ino == cgroup['inode']
    assert Path(f"/proc/{process['pid']}/cgroup").read_text().strip() == (
        '0::/' + str(group.relative_to('/sys/fs/cgroup')))
    uid = int(next(line.split()[1] for line in Path(f"/proc/{process['pid']}/status").read_text().splitlines()
                   if line.startswith('Uid:')))
    assert uid == settings['bounded_host']['uid_base'] + context['position']
    return dict(resource_id=name, pid=process['pid'], start_time_ticks=process['start_time_ticks'],
                uid=uid, namespace_inode=network['namespace_inode'], disk=[disk.st_dev, disk.st_ino, disk.st_size],
                cgroup_inode=cgroup['inode'])


def audit_absent(settings, identity, ticks):
    name = identity['resource_id']
    assert ticks(identity['pid']) != identity['start_time_ticks'], 'original VMM is still alive'
    assert not (Path(settings['chroot_base']) / Path(settings['firecracker']).name / name).exists()
    assert not (Path('/run/netns') / name).exists()
    assert not (Path(settings['host_resources']['cgroup_parent']) / name).exists()
    # Namespace inode alone is not a permanent identity: later namespaces can
    # reuse it. The NodeHost release witness plus original PID/start and named
    # resources are checked; a separate old-holder experiment is not claimed.


def validate_stale_report(report, count):
    if (not isinstance(report, dict) or report.get('version') != 1
            or any(report.get(field) is not True for field in (
                'context_rejected', 'release_rejected', 'all_actions_rejected'))
            or type(report.get('checked_actions')) is not int or report['checked_actions'] != count):
        raise AssertionError('late host-action probe failed or incomplete')


def exercise(args):
    from server_process_crash import free_loopback_port
    from _runtime_services import RuntimeServices
    from sdk_c02_acceptance import processes_in_uid_range

    assert sys.platform == 'linux' and os.geteuid() == 0
    assert importlib.metadata.version('e2b') == '2.51.0'
    require_bounded_host_tools()
    settings = private_json(args.native_config)
    host_config, host_digest = host.load_config(args.host_config)
    pool, peak, inodes = validate_configuration(settings, host_config, args.rounds, args.wait_seconds)
    output = host.trusted_path(args.output_dir)
    assert output.is_dir() and not output.stat().st_mode & 0o077 and not list(output.iterdir())
    for root in (Path(settings['database']).parent, Path(settings['node_state_dir']), Path(settings['chroot_base'])):
        host.trusted_path(root)
        assert output != root and root not in output.parents and output not in root.parents
    assert not Path(settings['database']).exists(), 'requires a fresh reviewed deployment; never deletes an old DB'
    assert not processes_in_uid_range(settings['uid_base'], settings['uid_base'] + 1)
    for name, sha in ((args.stale_probe, args.stale_probe_sha256),):
        host.artifact_fingerprint({'path': str(name), 'sha256': sha}, executable=True)
    preflight = host.inspect_host(host_config, host_digest)
    write_private(output / 'preflight.json', preflight)
    assert preflight['prerequisites_passed'], 'fixed host preflight failed'
    filesystem = os.statvfs(settings['chroot_base'])
    assert filesystem.f_favail >= 2 * inodes, 'fresh physical inode peak unavailable'
    catalog = settings['templates']
    assert catalog['aliases'].get('base') in {
        binding['template_id'] for binding in catalog['templates'] if binding['owner'] is None
    }, 'the two-user probe requires its explicit original shared base template'
    key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    url = 'http://127.0.0.1:' + str(free_loopback_port())
    root = Path(settings['database']).parent.parent
    sizes = [binding['kernel']['size_bytes']+binding['rootfs']['size_bytes'] for binding in catalog['templates']]
    registry = dict(root=str(root/'registry'),max_templates_global=len(sizes),max_templates_per_tenant=len(sizes),
        max_artifact_bytes_global=sum(sizes),max_artifact_bytes_per_tenant=sum(sizes),validation_timeout_secs=60)
    services = RuntimeServices.prepare(args,root=root,native_pools=[settings],registered_catalogs=[catalog],
        key=key,public_address=url.removeprefix('http://'),record=args.service_cgroup_record,
        job_count=2,registry_config=registry,extra_credentials=[{'tenant':2,'secret':foreign_key}])
    try:
        services.start()
        services.wait_ready()
        report = exercise_on_services(services,key,foreign_key,output,
            stale_probe=args.stale_probe,stale_probe_sha256=args.stale_probe_sha256,
            rounds=args.rounds,wait_seconds=args.wait_seconds)
        for name in host.ARTIFACTS:
            host.artifact_fingerprint(host_config['artifacts'][name], executable=name in ('firecracker', 'jailer'))
        print(json.dumps(dict(result=report['result'],evidence_root=str(output),
            report_sha256=digest(output/'report.json'))),flush=True)
    finally:
        services.stop_all()


def exercise_on_services(services,key,foreign_key,evidence_root,*,stale_probe,
                         stale_probe_sha256,rounds=6,wait_seconds=30):
    """Run the ORIGINAL churn assertions on one already started DK assembly.

    The caller owns provisioning, the sole execution manifest and final service
    cleanup. Planned offline audits/restarts keep the original configurations;
    there is no second RuntimeServices or synthetic successful host action.
    """
    from server_process_crash import fetch, process_start_ticks
    from _center_host_services import local_binding
    from sdk_c02_acceptance import processes_in_uid_range
    from sdk_server_smoke import options
    from tools.runtime_create_recovery import terminal_result
    from e2b import Sandbox

    assert sys.platform == 'linux' and os.geteuid() == 0
    assert importlib.metadata.version('e2b') == '2.51.0'
    require_bounded_host_tools()
    if len(services.native_pools) != 1 or key == foreign_key:
        raise ValueError('one original runtime and two distinct tested tenants required')
    settings = services.native_pools[0]
    pool,peak,inodes = validate_runtime_configuration(settings,rounds,wait_seconds)
    retained_slots = retention_slots(services,rounds)
    output = host.trusted_path(evidence_root)
    assert output.is_dir() and not output.stat().st_mode & 0o077
    assert not any((output/name).exists() for name in ('node-config.json','old-actions.json','report.json'))
    for root in (Path(settings['database']).parent,Path(settings['node_state_dir']),Path(settings['chroot_base'])):
        assert output != root and root not in output.parents and output not in root.parents
    host.artifact_fingerprint({'path':str(stale_probe),'sha256':stale_probe_sha256},executable=True)
    services.processes.observe()  # Actual same-role identities, not stale Popen handles.
    node_file = output/'node-config.json'
    write_private(node_file,node_configuration(settings))
    url = services.sdk_url
    owner = options(key, url, url)
    owner['request_timeout'] = 180
    victim_owner = options(foreign_key,url,url) | {'request_timeout':180}
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    sampler, live = FilePeaks(settings['database']), []
    report = dict(schema_version=2, scope='finite_single_runtime_bounded_churn',
                  source=host.source_revision(),
                  center_sha256=digest(services.processes.bindings['center']['binary']),
                  host_sha256=digest(services.processes.bindings['host-0']['binary']),
                  center_config_sha256=digest(services.processes.bindings['center']['config']),
                  host_config_sha256=digest(services.processes.bindings['host-0']['config']),
                  stale_probe_sha256=stale_probe_sha256,
                  probe_sha256=digest(__file__), physical_peak_bytes=peak, physical_peak_inodes=inodes,
                  rounds=rounds, observations=[], identities=[], mappings=[], authority_observations=[],
                  stale_actions=None, result='incomplete',
                  limits='No old namespace-FD holder injection, unknown-work experiment, two-pool builds, power loss, or formal proof.')
    started, retired, old, old_actions, positions = [], [], None, [], []

    def sample(label):
        value = database_snapshot(settings['database'])
        assert_budget(value, settings)
        value['host_metadata'] = metadata_usage(settings)
        assert value['host_metadata']['entries'] <= pool['host_metadata_inodes']
        assert value['host_metadata']['logical_file_bytes'] <= pool['host_metadata_bytes']
        report['observations'].append(dict(label=label, **value))
        return value

    def wait_retired(ident):
        deadline = time.monotonic() + wait_seconds
        while True:
            snapshot = database_snapshot(settings['database'])
            assert_budget(snapshot, settings)
            if ident not in snapshot['phases']:
                return snapshot
            assert time.monotonic() < deadline, 'original terminal metadata did not retire'
            time.sleep(0.1)  # Poll an actual state transition; no guessed success.

    def survivor_ok(sandbox, context, identity):
        assert live_identity(settings, context, process_start_ticks) == identity
        assert sandbox.commands.run('printf bounded-survivor', user='ubuntu').stdout == 'bounded-survivor'
        assert sandbox.files.read('/tmp/sandproof-bounded-survivor') == 'original'

    def audit_pair(label):
        # The two checked authorities remain diagnostic sources, never release
        # decisions. VMM processes are independent of these service restarts.
        services.processes.stop('center')
        services.processes.stop('host-0')
        try:
            center, native = services.processes.audit('center'), services.processes.audit('host-0')
        finally:
            services.processes.start('host-0')
            services.processes.start('center')
            services.wait_ready(for_admission=False)
        bounds = services.center_config['bounds']
        assert len(center['sandboxes']) <= bounds['max_sandboxes']
        assert all(len(record['operations']) <= bounds['operations_per_sandbox']
                   for record in center['sandboxes'])
        assert len(center['retention_lanes']) == len(native['lanes']) == retained_slots
        report['authority_observations'].append(dict(label=label,
            center_retention_lanes=center['retention_lanes'],host_retention_lanes=native['lanes'],
            center_records=len(center['sandboxes']),host_operations=len(native['operations']),
            sandbox_highwater=center['sandbox_highwater'],operation_highwater=center['operation_highwater']))
        return center,native

    def original_mapping(sandbox):
        center,native = audit_pair('mapped-'+sandbox.sandbox_id)
        rows = [record for record in center['sandboxes']
                if record['create']['identity']['sandbox_id'] == sandbox.sandbox_id]
        assert len(rows) == 1 and not rows[0]['released']
        identity = rows[0]['create']['identity']
        binding = local_binding(native,sandbox.sandbox_id,expected_node=services.host_configs[0]['inbox']['node'])
        report['mappings'].append(dict(create=identity,local=binding))
        return 'sb-'+settings['namespace']+'-'+str(binding['sandbox']),binding['sandbox'],identity

    def exact_retired(identity, survivor_identity=None):
        deadline = time.monotonic()+wait_seconds
        while True:
            center,native = audit_pair('terminal-'+identity['sandbox_id'])
            terminal = terminal_result(center,native,identity)
            if terminal and terminal['status'] == 'retired_terminal_slot':
                assert terminal['center_retired_through'] == terminal['host_retired_through'] == identity['request_window']
                if survivor_identity is not None:
                    # A live peer must retain its exact original incarnation,
                    # never get retired merely to recycle the victim's slot.
                    peers = [row for row in center['sandboxes']
                             if row['create']['identity'] == survivor_identity]
                    assert len(peers) == 1 and not peers[0]['released']
                    assert survivor_identity['retention_slot'] != identity['retention_slot']
                    for lanes in (center['retention_lanes'],native['lanes']):
                        lane = next(row for row in lanes if row['retention_slot'] == survivor_identity['retention_slot'])
                        assert lane['retired_through'] == survivor_identity['request_window']-1
                return terminal
            assert time.monotonic()<deadline,'original exact terminal slot did not retire on both authorities'
            time.sleep(.1)

    try:
        sampler.thread.start()
        initial = sample('fresh')
        assert initial['counts']['sandboxes'] == 0,'shared DK predecessors did not reclaim original native records'
        survivor = Sandbox.create(template='base', timeout=600, allow_internet_access=False, **owner)
        live.append(survivor)
        survivor_native,survivor_local,survivor_create = original_mapping(survivor)
        survivor.files.write('/tmp/sandproof-bounded-survivor', 'original')
        snapshot = sample('survivor')
        survivor_context = context_for(snapshot, survivor_local)
        survivor_identity = live_identity(settings, survivor_context, process_start_ticks)
        report['identities'].append(survivor_identity)
        started.append(survivor.sandbox_id)
        victim_slot,previous_window = None,None
        for index in range(rounds):
            victim = Sandbox.create(template='base', timeout=600, allow_internet_access=False, **victim_owner)
            live.append(victim)
            victim_native,victim_local,victim_create = original_mapping(victim)
            if index == 0:
                victim_slot = victim_create['retention_slot']
            else:
                assert victim_create['retention_slot'] == victim_slot
                assert victim_create['request_window'] == previous_window+1
            previous_window = victim_create['request_window']
            assert fetch(opener,url,'GET','/sandboxes/'+survivor.sandbox_id,foreign_key)[0] == 404
            assert fetch(opener,url,'GET','/sandboxes/'+victim.sandbox_id,key)[0] == 404
            snapshot = sample('created-' + str(index))
            context = context_for(snapshot, victim_local)
            identity = live_identity(settings, context, process_start_ticks)
            report['identities'].append(identity)
            assert context['position'] != survivor_context['position']
            positions.append(context['position'])
            started.append(victim.sandbox_id)
            if index == 0:
                old, old_actions = context, capture_actions(settings['database'], victim_native)
                assert len(old_actions) >= 4, 'capture real accepted host actions before retirement'
            elif index == 1:
                assert context['position'] == old['position'] and context['binding']['vm'] != old['binding']['vm']
                services.processes.stop('host-0')
                before = tree_records((settings['node_state_dir'], Path(settings['chroot_base']) / '.claims'))
                action_file = output / 'old-actions.json'
                write_private(action_file, dict(version=1, old_context=old, actions=old_actions))
                result = subprocess.run([str(stale_probe), str(node_file), str(action_file)],
                                        capture_output=True, timeout=30, check=False,
                                        env={'PATH': '/usr/bin:/bin', 'LANG': 'C'})
                assert result.returncode == 0 and len(result.stdout) <= 65536, 'late host-action driver failed'
                stale = json.loads(result.stdout, object_pairs_hook=host.unique_keys)
                validate_stale_report(stale, len(old_actions))
                assert tree_records((settings['node_state_dir'], Path(settings['chroot_base']) / '.claims')) == before
                assert live_identity(settings, context, process_start_ticks) == identity
                assert live_identity(settings, survivor_context, process_start_ticks) == survivor_identity
                report['stale_actions'] = stale
                services.processes.start('host-0')
                services.wait_ready(for_admission=False)
                survivor_ok(survivor, survivor_context, survivor_identity)
                assert victim.commands.run('printf new-incarnation').stdout == 'new-incarnation'
                sample('same-config-restart-and-late-actions')
            victim_killed = Sandbox.kill(victim.sandbox_id, **victim_owner)
            assert victim_killed
            live.remove(victim)
            if index == 0:
                extra = capture_actions(settings['database'], victim_native)
                if extra:
                    old_actions = extra
            wait_retired(victim_native)
            exact_retired(victim_create,survivor_create)
            sample('retired-' + str(index))
            audit_absent(settings, identity, process_start_ticks)
            retired.append(victim.sandbox_id)
            assert fetch(opener, url, 'GET', '/sandboxes/' + victim.sandbox_id, foreign_key)[0] == 404
            assert fetch(opener, url, 'DELETE', '/sandboxes/' + victim.sandbox_id, foreign_key)[0] == 404
            survivor_ok(survivor, survivor_context, survivor_identity)
        assert len(set(positions)) == 1, 'churn did not actually reuse the same physical position'
        assert context['binding']['vm'] > 2, 'permanent VM identity never crossed the physical position count'
        survivor_killed = Sandbox.kill(survivor.sandbox_id, **owner)
        assert survivor_killed
        live.remove(survivor)
        wait_retired(survivor_native)
        exact_retired(survivor_create)
        retired.append(survivor.sandbox_id)
        audit_absent(settings, survivor_identity, process_start_ticks)
        final = sample('all-retired')
        assert final['counts']['sandboxes'] == 0 and final['counts']['outbox'] == 0
        assert final['checkpoint']['generation'] > initial['checkpoint']['generation']
        assert final['checkpoint']['input_cut'] > initial['checkpoint']['input_cut']
        assert final['checkpoint']['deleted_through'] > initial['checkpoint']['deleted_through']
        services.processes.stop('host-0')
        services.processes.stop('center')
        records_before = tree_records((settings['node_state_dir'], Path(settings['chroot_base']) / '.claims'))
        services.processes.start('host-0')
        services.processes.start('center')
        services.wait_ready(for_admission=False)
        reopened = sample('all-retired-reopen')
        assert reopened['counts']['sandboxes'] == 0
        assert_reopen_frontiers(final['highwaters'], reopened['highwaters'])
        services.processes.stop('host-0')
        assert tree_records((settings['node_state_dir'], Path(settings['chroot_base']) / '.claims')) == records_before
        assert not processes_in_uid_range(settings['uid_base'], settings['uid_base'] + 1)
        sampler.stop()
        for name, value in sampler.peaks.items():
            assert value <= settings['maintenance']['capacity']['max_' + name + '_bytes']
        assert not any(Path(settings['host_resources']['cgroup_parent']).glob(f"vm-{settings['namespace']}-*"))
        metadata = tree_records((settings['node_state_dir'], Path(settings['chroot_base']) / '.claims'))
        report.update(vms_created=len(started), vms_retired=len(retired),
                      reused_position=positions[0], observed_file_peaks=sampler.peaks,
                      final_host_metadata_files=len(metadata),
                      final_host_metadata_logical_bytes=sum(value[3] for value in metadata.values()
                                                            if value[2] == stat.S_IFREG))
        report['observed_host_metadata_peaks'] = {
            key: max(item['host_metadata'][key] for item in report['observations'])
            for key in ('entries', 'logical_file_bytes', 'allocated_bytes')}
        assert report['final_host_metadata_files'] <= pool['host_metadata_inodes']
        assert report['final_host_metadata_logical_bytes'] <= pool['host_metadata_bytes']
        services.processes.start('host-0')
        services.wait_ready(for_admission=False)
        services.processes.observe()
        report['result'] = 'passed'
    except BaseException as error:
        report['failure_type'] = type(error).__name__
        raise
    finally:
        if sampler.thread.ident is not None and not sampler.stopped.is_set():
            try:
                sampler.stop()
            except Exception as error:
                # Preserve the original failure and still write partial evidence.
                report['sampler_failure_type'] = type(error).__name__
                report['result'] = 'incomplete'
        report['unreclaimed_known_sandboxes'] = [sandbox.sandbox_id for sandbox in live]
        write_private(output / 'report.json', report)
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    from _runtime_services import add_arguments
    add_arguments(parser)
    for name in ('native-config', 'host-config', 'stale-probe', 'output-dir'):
        parser.add_argument('--' + name, required=True, type=Path)
    for name in ('stale-probe-sha256',):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--rounds', type=int, default=6)
    parser.add_argument('--wait-seconds', type=int, default=30)
    exercise(parser.parse_args())
