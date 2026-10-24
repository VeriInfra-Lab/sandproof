"""Unique protected-role fixture for the real center/Redis/private-host chain.

This is test assembly, not another controller, public entry or health authority.
The original Rust configurations validate the whole business envelope and host
SQL/physical bounds. Empty initial availability is intentional: only actual
authenticated host observation can make a node eligible for dispatch.
"""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess

from tools import service_cgroup
from tools.ci_e2e_artifacts import tls_material_files
from tools.ci_e2e_prepare import bounded_json_command
from tools.check_kvm_host import trusted_path
from tools.system_acceptance import private_path
from tools.runtime_create_recovery import find_runtime_create, original_create_identity, terminal_result


BLOCK_BYTES = 16 * 1024**2
REDIS_MEMORY_BYTES = 128 * 1024**2
CERTIFICATE_BYTES = 1024**2
OPTIONS_BYTES = 4096
OPERATIONS = 4
# Center terminal-result retention; independent of Host/build/Guest retention.
RESULT_RETENTION_MS = 300_000
HOST_FAULT_VARIABLES = frozenset({
    'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION', 'SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION',
    'SANDPROOF_TEST_EXIT_AFTER_GUEST_INIT', 'SANDPROOF_TEST_FAIL_RECLAIM_STOP_SANDBOX',
    'SANDPROOF_TEST_FAIL_RECLAIM_PREPARE_SANDBOX', 'SANDPROOF_TEST_EXIT_AFTER_VM_CLAIM',
    'SANDPROOF_TEST_EXIT_AFTER_TEMPLATE_START', 'SANDPROOF_TEST_STOP_GATE',
})


def identities(center_uid, center_gid, redis_uid, redis_gid, *, uid_base):
    pairs = {role: service_cgroup.service_identity(uid, gid) for role, uid, gid in (
        ('center', center_uid, center_gid), ('redis', redis_uid, redis_gid))}
    if (None in pairs.values() or pairs['center'][0] == pairs['redis'][0]
            or type(uid_base) is not int or any(pair[0] >= uid_base for pair in pairs.values())):
        raise ValueError('independent explicit service identities outside VM UID pools required')
    return {'host': (0, 0), **pairs}


def tls_files(root, role):
    if role not in ('center', 'host'):
        raise ValueError('exact TLS service role required')
    peer = 'host' if role == 'center' else 'center'
    root = Path(root)
    return dict(identity_chain=[str(root / (role + '.der'))],
                private_key=str(root / (role + '-key.der')), roots=[str(root / 'ca.der')],
                **{peer + '_leaves': [str(root / (peer + '.der'))]})


def test_credentials(key,extra_credentials=None):
    """Explicit fixture owners only; no caller-controlled deployment override."""
    if extra_credentials is None:
        extra_credentials=[]
    if type(extra_credentials) is not list or len(extra_credentials)>15:
        raise ValueError('finite explicit additional test credentials required')
    values=[dict(tenant=1,secret=key),*copy.deepcopy(extra_credentials)]
    owners,secrets=set(),set()
    for value in values:
        if (type(value) is not dict or set(value)!={'tenant','secret'}
                or type(value['tenant']) is not int or not 0<value['tenant']<2**64
                or type(value['secret']) is not str or not 0<len(value['secret'])<=256
                or not value['secret'].isascii() or any(not 33<=ord(char)<=126 for char in value['secret'])
                or value['tenant'] in owners or value['secret'] in secrets):
            raise ValueError('distinct bounded test owner IDs and credentials required')
        owners.add(value['tenant'])
        secrets.add(value['secret'])
    return values


def render(native_pools, *, registered_catalogs, deployment, center_root, host_tls_roots, host_addresses,
           public_address, artifact_address, redis_address, password, key, job_count,
           registry, versions, capture_settings, private_builds=None, private_build_boundary=None,extra_credentials=None):
    """Render one explicit protected product configuration per role.

    This function neither computes Rust canonical descriptor digests nor models
    required-block/SQL reservations. Product initialize must reject insufficient
    approved capacities; this fixture never increases them to force success.
    """
    if (not native_pools or len(native_pools) != len(host_addresses)
            or len(native_pools) != len(host_tls_roots) or len(native_pools) != len(registered_catalogs)
            or len(native_pools) > 2
            or type(job_count) is not int or not 0 < job_count <= 10000):
        raise ValueError('explicit finite test cohort and registered host roles required')
    if (type(capture_settings) is not list or len(capture_settings) != len(native_pools)
            or any(type(value) is not dict for value in capture_settings)):
        raise ValueError('one explicit protected capture profile per original host required')
    credentials=test_credentials(key,extra_credentials)
    center_root = Path(center_root)
    center_id = 'center-main'
    catalog = {'templates': [], 'aliases': {}, 'compute_limits': {}}
    template_ids = {binding['template_id'] for registered in registered_catalogs
                    for binding in registered['templates']}
    hosts, nodes, registrations = [], [], []
    platform = dict(instances=0, cpu=0, memory_bytes=0, disk_bytes=0)
    tenant = dict(platform)
    max_records = job_count + sum(pool['max_active_global'] for pool in native_pools)
    if type(max_records) is not int or not 0 < max_records <= 2**16 - 1:
        raise ValueError('existing inbox capacity must fit the mandatory retention slot count')
    expected_roles = ('runtime',) if len(native_pools) == 1 else ('runtime','builder')
    for index, (source, registered, address, tls_root) in enumerate(zip(
            native_pools, registered_catalogs, host_addresses, host_tls_roots)):
        if any(name in source for name in ('bind','domain','credentials','request_admission',
                'template_registry','internal_build_vms','build_retry_windows')):
            raise ValueError('new native inputs required; retired public host configuration is not upgraded')
        native = copy.deepcopy(source)
        native_catalog = native['templates']
        pools = [pool for pool in native['bounded_host']['physical_storage']['pools']
                 if pool['namespace'] == native['namespace']]
        if len(pools) != 1 or pools[0]['role'] != expected_roles[index]:
            raise ValueError('exact original physical runtime/builder cohort required')
        role = pools[0]['role']
        node = dict(deployment_id=deployment, node_id='host-' + role, node_generation=1)
        entries = [dict(revision=value['template_id'] + '-' + value['revision'],
                       descriptor=copy.deepcopy(value)) for value in registered['templates']]
        if any(len(value['revision']) > 64 for value in entries):
            raise ValueError('original static revision exceeds protocol identifier bound')
        registrations.extend(dict(revision=entry['revision'], template_id=entry['descriptor']['template_id'])
                             for entry in entries)
        catalog['templates'].extend(copy.deepcopy(registered['templates']))
        for alias, target in registered['aliases'].items():
            name = alias if index == 0 else 'builder-' + alias
            # The merged name may already resolve directly to this descriptor
            # (builder + base -> builder-base). Rust rejects aliases that shadow
            # any template ID, including a redundant alias to itself.
            if name in template_ids:
                if name != target:
                    raise ValueError('static template alias shadows another template identity')
                continue
            if name in catalog['aliases']:
                raise ValueError('static template aliases collide')
            catalog['aliases'][name] = target
        for name, value in native_catalog['compute_limits'].items():
            catalog['compute_limits'][name] = catalog['compute_limits'].get(name, 0) + value
        limits = dict(instances=native['max_active_global'],
            cpu=native_catalog['compute_limits']['max_vcpus_global'],
            memory_bytes=native_catalog['compute_limits']['max_memory_bytes_global'],
            disk_bytes=native['max_disk_bytes_global'] - native['cleanup_reserve_bytes'])
        personal = dict(instances=native['max_active_per_tenant'],
            cpu=native_catalog['compute_limits']['max_vcpus_per_tenant'],
            memory_bytes=native_catalog['compute_limits']['max_memory_bytes_per_tenant'],
            disk_bytes=native['max_disk_bytes_per_tenant'])
        for name in platform:
            platform[name] += limits[name]
            tenant[name] += personal[name]
        transfers = native['data_transfer']['max_transfers']
        matching_caches = [cache for cache in native['bounded_host']['physical_storage']['host_template_caches']
                           if cache['namespace'] == native['namespace']]
        if len(matching_caches) != 1:
            raise ValueError('explicit original physical cache reservation required')
        cache = copy.deepcopy(matching_caches[0])
        artifact_tls = tls_files(tls_root, 'host')
        artifact_tls['artifact_leaves'] = artifact_tls.pop('center_leaves')
        capture = copy.deepcopy(capture_settings[index])
        hosts.append(dict(native=native, inbox=dict(node=node,
            center=dict(deployment=deployment, center=center_id), host_epoch=None,
            max_sessions=max_records, max_operations_per_session=8,
            max_options_bytes=OPTIONS_BYTES, max_profile_bytes=65536, max_manifest_bytes=262144),
            protected_bootstrap_path=str(Path(tls_root).parent/'host-bootstrap.json'),
            source_observation=dict(max_work_bytes=65536,max_configuration_bytes=65536,
                max_executable_bytes=67108864,max_kernel_bytes=268435456,
                max_disk_bytes=capture['disk_copy_bytes'],
                max_material_bytes=capture['material_project']['hard_bytes']),
            capture=capture,
            internal=dict(bind=address, max_connections=transfers + 4,
                handshake_timeout_ms=2000, request_timeout_ms=10000, tls=tls_files(tls_root, 'host')),
            template_cache=cache,
            artifact_source=dict(address=artifact_address, server_name='center.test', tls=artifact_tls,
                max_connections=cache['max_downloads'], handshake_timeout_ms=2000,
                request_timeout_ms=10000, download_timeout_ms=900000, idle_timeout_ms=30000)))
        # No positive free/health/cache claim is manufactured by configuration.
        nodes.append(dict(registration=dict(identity=node, role=role, limit=limits, mode='accepting',
            healthy_until_ms=0, free={name: 0 for name in limits}, templates=[],
            protocol_version=versions['internal_protocol'], retention_slots=max_records), address=address,
            server_name='host.test', peer_certificates_der=[str(center_root / ('tls/host-' + role + '.der'))]))
    if len({entry['template_id'] for entry in registrations}) != len(registrations):
        raise ValueError('static template identities collide across registered hosts')
    runtime = native_pools[0]
    transfers = runtime['data_transfer']['max_transfers']
    center_transfer = copy.deepcopy(runtime['data_transfer'])
    # Both public runtime and private builder files pass this shared gateway.
    # Carry the already approved host file envelopes, while each destination
    # still enforces its own native limit and center concurrency stays fixed.
    for native in native_pools[1:]:
        for field in ('file_upload_bytes', 'file_download_bytes'):
            center_transfer[field] = max(center_transfer[field], native['data_transfer'][field])
    tls_root = center_root / 'tls'
    common_tls = dict(certificate_chain_der=[str(tls_root / 'center.der')],
        private_key_der=str(tls_root / 'center-key.der'), trusted_roots_der=[str(tls_root / 'ca.der')])
    center = dict(deployment_id=deployment, center_id=center_id, bind=public_address, domain='example.invalid',
        credentials=credentials, data_token_key=str(center_root / 'token.key'),
        max_timeout_secs=runtime['max_timeout_secs'], max_lifetime_secs=runtime['max_lifetime_secs'],
        startup_timeout_secs=runtime['startup_timeout_secs'], reply_timeout_ms=25000,
        poll_interval_ms=100, expiry_scan_ms=runtime['expiry_scan_seconds']*1000,
        public_ipv4_enabled=runtime['public_ipv4_enabled'],
        request_admission=dict(max_pending_requests=transfers + 4, control_reserve=4),
        data_transfer=center_transfer,
        redis=dict(address=redis_address, username='default', password=password,
            max_block_bytes=BLOCK_BYTES, memory_bytes=REDIS_MEMORY_BYTES,
            operation_timeout_ms=1000, writer_root=str(center_root / 'writer')),
        tls=dict(**common_tls, max_connections=transfers + 4,
            handshake_timeout_ms=2000, request_timeout_ms=10000),
        node_observation=dict(health_window_ms=runtime['expiry_scan_seconds']*1000+5000,
                              max_clock_skew_ms=100),
        artifact_service=dict(bind=artifact_address, server_name='center.test', tls=copy.deepcopy(common_tls),
            max_connections=4, handshake_timeout_ms=2000, request_timeout_ms=10000,
            stream_timeout_ms=900000),
        bounds=dict(max_nodes=len(nodes), max_templates_per_node=max(host['template_cache']['max_entries'] for host in hosts),
            max_sandboxes=max_records, operations_per_sandbox=OPERATIONS,
            max_options_bytes=OPTIONS_BYTES, result_retention_ms=RESULT_RETENTION_MS, platform=platform),
        tenants=[dict(tenant=str(value['tenant']),limit=copy.deepcopy(tenant)) for value in credentials], nodes=nodes,
        template_registry=copy.deepcopy(registry), metadata_budget=dict(max_certificate_bytes=CERTIFICATE_BYTES),
        private_builds=copy.deepcopy(private_builds),
        private_build_boundary=copy.deepcopy(private_build_boundary),
        templates=catalog, template_registrations=registrations)
    return center, hosts


def redis_configuration(root, *, port, password):
    if type(port) is not int or not 1 <= port <= 65535 or not password.isalnum():
        raise ValueError('fixed loopback Redis endpoint and generated test credential required')
    root = Path(root)
    return (f'bind 127.0.0.1\nport {port}\nprotected-mode yes\nrequirepass {password}\n'
        f'appendonly yes\nappendfsync always\nmaxmemory {REDIS_MEMORY_BYTES}\nmaxmemory-policy noeviction\n'
        'no-appendfsync-on-rewrite no\naof-load-truncated no\nauto-aof-rewrite-percentage 100\n'
        # The exact role observer verifies the approved argv. Redis must not
        # replace it with a mutable process title after startup.
        'auto-aof-rewrite-min-size 16mb\nset-proc-title no\nsave ""\n'
        f'dir {root}\nlogfile {root / "redis.log"}\n')


def role_facts(directory, config, *, uid, gid):
    """Observe actual protected role identities, not claimed configuration."""
    directory, config = Path(directory), Path(config)
    before = directory.lstat()
    file = config.lstat()
    if (not stat.S_ISDIR(before.st_mode) or (before.st_uid, before.st_gid) != (uid, gid)
            or before.st_mode & 0o7777 != 0o700 or config.parent != directory
            or not stat.S_ISREG(file.st_mode) or file.st_nlink != 1
            or (file.st_uid, file.st_gid) != (uid, gid) or file.st_mode & 0o7777 != 0o600
            or not 0 < file.st_size <= 1024**2):
        raise ValueError('actual service role directory or protected configuration changed')
    identity = lambda info: (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
    descriptor = os.open(config, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, 'rb') as source:
        if identity(os.fstat(source.fileno())) != identity(file):
            raise ValueError('role configuration inode changed')
        checksum = hashlib.file_digest(source, 'sha256').hexdigest()
        if (identity(os.fstat(source.fileno())) != identity(file)
                or identity(config.lstat()) != identity(file)
                or (directory.stat().st_dev, directory.stat().st_ino) != (before.st_dev, before.st_ino)):
            raise ValueError('role configuration changed during observation')
    return dict(uid=uid, gid=gid, config_sha256=checksum, directory=dict(
        device=before.st_dev, inode=before.st_ino, uid=uid, gid=gid, mode=0o700))


def write_role(path, content, *, uid, gid):
    """Create once, with explicit role ownership; never repair existing files."""
    raw = json.dumps(content, separators=(',', ':'), ensure_ascii=False).encode() if isinstance(content, dict) else content
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as output:
        output.write(raw)
        os.fchown(output.fileno(), uid, gid)
        os.fsync(output.fileno())


def stage_role_directories(parent, name, hosts, *, service_ids):
    """Create only bounded role roots; never relax the private evidence root.

    The enclosing run keeps this newly allocated identity on any uncertainty.
    No existing directory is repaired and no automatic deletion is performed.
    Actual storage peaks/available bytes remain the original physical profile's
    responsibility, including every role and immutable artifact copy.
    """
    if (type(name) is not str or re.fullmatch(r'sp292-[a-f0-9]{16,64}', name) is None
            or set(service_ids) != {'center','redis','host'} or service_ids['host'] != (0,0)
            or service_cgroup.service_identity(*service_ids['center']) is None
            or service_cgroup.service_identity(*service_ids['redis']) is None
            or service_ids['center'][0] == service_ids['redis'][0]
            or not 1 <= len(hosts) <= 2):
        raise ValueError('exact finite staged roles and original identities required')
    parent = trusted_path(Path(parent))
    metadata = parent.lstat()
    host_roots = [private_path(path,directory=True) for path in hosts]
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0
            or metadata.st_mode & 0o7777 != 0o711
            or len(set(host_roots)) != len(host_roots)
            or any(root.stat().st_dev != metadata.st_dev for root in host_roots)):
        raise ValueError('protected traversable same-filesystem service parent required')
    common = parent / name
    common.mkdir(mode=0o711)
    # Caller umask may be 077; only this newly created, exactly bounded common
    # parent is traversable. Per-role children remain 0700 with distinct owners.
    common.chmod(0o711)
    result = dict(common=common,hosts=host_roots)
    for role in ('center','redis'):
        child = common / role
        child.mkdir(mode=0o700)
        os.chown(child,*service_ids[role])
        result[role] = child
    return result


def stage_tls(material, expected_sha256, *, center, hosts, center_identity):
    """Copy preapproved distinct dual-usage material; never generate a CA.

    All source identities and hashes are checked before filesystem effects.
    Product TLS still verifies real EKU/SAN and authenticated node bindings.
    This copy step cannot declare TLS/Redis/VM success or a cleanup result.
    """
    if not 1 <= len(hosts) <= 2 or service_cgroup.service_identity(*center_identity) is None:
        raise ValueError('explicit approved center and finite root-host roles required')
    roles = ('runtime',) if len(hosts) == 1 else ('runtime','builder')
    names = tls_material_files(roles)
    if set(expected_sha256) != set(names):
        raise ValueError('complete exact fixed TLS material receipt required')
    material = private_path(material,directory=True)
    data = {}
    for name in names:
        source = private_path(material / name)
        before = source.lstat()
        if before.st_nlink != 1 or not 0 < before.st_size <= 64*1024:
            raise ValueError('bounded independent private TLS source required')
        descriptor = os.open(source,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
        with os.fdopen(descriptor,'rb') as input:
            opened = os.fstat(input.fileno())
            content = input.read(64*1024+1)
            identity = lambda value:(value.st_dev,value.st_ino,value.st_size,value.st_mtime_ns,value.st_ctime_ns)
            if (identity(opened) != identity(before) or identity(os.fstat(input.fileno())) != identity(before)
                    or identity(source.lstat()) != identity(before)
                    or hashlib.sha256(content).hexdigest() != expected_sha256[name]):
                raise ValueError('approved service TLS material changed')
            data[name] = content
    if len({data['host-' + role + '.der'] for role in roles}) != len(roles):
        raise ValueError('one fixed leaf per registered node required')
    destination_roots = [Path(center),*(Path(root) for root in hosts)]
    owners = []
    for index, root in enumerate(destination_roots):
        metadata = root.lstat()
        if (root.resolve(strict=True) != root or not stat.S_ISDIR(metadata.st_mode)
                or metadata.st_mode & 0o7777 != 0o700
                or (index == 0 and (metadata.st_uid,metadata.st_gid) != center_identity)
                or (index > 0 and metadata.st_uid != 0)):
            raise ValueError('exact already-protected role root required')
        owners.append((metadata.st_uid,metadata.st_gid))
    if len(set(destination_roots)) != len(destination_roots):
        raise ValueError('independent TLS role roots required')
    center_files = {name:data[name] for name in ('ca.der','center.der','center-key.der')}
    center_files.update({name:data[name] for name in names
                         if name.startswith('host-') and not name.endswith('-key.der')})
    files = [center_files,*[dict(
        **{'ca.der':data['ca.der'],'center.der':data['center.der']},
        **{'host.der':data['host-' + role + '.der'],'host-key.der':data['host-' + role + '-key.der']})
        for role in roles]]
    for root,contents,owner in zip(destination_roots,files,owners):
        tls = root / 'tls'
        tls.mkdir(mode=0o700)
        os.chown(tls,*owner)
        for name,content in contents.items():
            write_role(tls/name,content,uid=owner[0],gid=owner[1])
    return [root / 'tls' for root in destination_roots]


def fault_environment(values):
    """Allow only explicit existing host fault controls, never inherited secrets."""
    if values is None:
        return {}
    if (type(values) is not dict or set(values) - HOST_FAULT_VARIABLES
            or any(type(value) is not str or not value or '\0' in value
                   or len(value.encode()) > 4096 for value in values.values())):
        raise ValueError('only bounded explicit original host fault variables are allowed')
    return dict(values)


def launch(binary, config, log, record, *, uid=0, gid=0, action='serve', env=None):
    if action not in ('serve', 'initialize', 'audit'):
        raise ValueError('one product command contract required')
    faults = fault_environment(env)
    if faults and (action != 'serve' or uid != 0 or gid != 0):
        raise ValueError('fault controls belong only to the private root host serve process')
    prefix = service_cgroup.prefix(record, **({} if uid == gid == 0 else dict(uid=uid, gid=gid)))
    with Path(log).open('ab') as output:
        return subprocess.Popen([*prefix, str(binary), action, '--config', str(config)],
            stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
            env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', **faults}, start_new_session=True)


def local_binding(audit, sandbox, *, expected_node):
    """Map a GLOBAL ID through the original checked inbox, never parse its suffix."""
    if (audit.get('node') != expected_node
            or type(audit.get('schema_version')) is not int or audit['schema_version'] != 2):
        raise ValueError('foreign or noncurrent host audit')
    rows = [row for row in audit['operations']
            if row['current'] is True and row['binding']['session'] == sandbox]
    if len(rows) != 1 or rows[0]['local'] is None:
        raise ValueError('exact current global/native mapping unknown')
    row = rows[0]
    if (row['binding']['node'] != expected_node
            or type(row['binding']['protocol_version']) is not int
            or row['binding']['protocol_version'] != 3
            or type(row['accepted_version']) is not int or not 0 < row['accepted_version'] < 2**64):
        raise ValueError('host audit identity mismatch')
    return row['local']


def command(binary, config, record, *, uid=0, gid=0, action='audit', timeout=30):
    """Explicit unique read-only audit or provisioning, never a legacy parser."""
    if action in ('audit', 'observe-center'):
        prefix = service_cgroup.prefix(record, **({} if uid == gid == 0 else dict(uid=uid, gid=gid)))
        return bounded_json_command([*prefix, str(binary), action, '--config', str(config)],
            {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, timeout=timeout,
            max_bytes=256 * 1024 if action == 'observe-center' else 16 * 1024**2)
    if action != 'initialize':
        raise ValueError('only explicit provisioning or checked original audit is allowed')
    process = launch(binary, config, Path(record).parent / 'initialize.log', record,
                     uid=uid, gid=gid, action='initialize')
    try:
        if process.wait(timeout=timeout) != 0:
            raise RuntimeError('explicit service initialization failed; retain original role inputs')
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)


def process_facts(process, binary, config, record, *, uid, gid, argv=None):
    """Observe exact live role PID/clock/executable/credentials in original leaf.

    No PID-only recovery, guessed healthy status, secret data or pass verdict is
    produced. The process must have finished the launcher's privilege drop and
    exec; early launch observations fail until the caller's bounded ready wait.
    """
    if process.poll() is not None:
        raise RuntimeError('original service child is no longer running')
    pid = process.pid
    proc = Path('/proc') / str(pid)
    start = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
    if start[0] == 'Z':
        raise RuntimeError('original service is a zombie')
    ticks = int(start[19])
    status = dict(line.split(':', 1) for line in (proc / 'status').read_text().splitlines() if ':' in line)
    if ([int(value) for value in status['Uid'].split()] != [uid]*4
            or [int(value) for value in status['Gid'].split()] != [gid]*4
            or (uid != 0 and (status['Groups'].split() or any(int(status[name].strip(), 16)
                for name in ('CapInh', 'CapPrm', 'CapEff', 'CapAmb'))))):
        raise ValueError('actual service privilege boundary differs from approved role')
    expected_argv = argv or (str(binary), 'serve', '--config', str(config))
    actual_argv = (proc / 'cmdline').read_bytes().rstrip(b'\0').split(b'\0')
    if actual_argv != list(map(os.fsencode, expected_argv)):
        raise ValueError('actual service argv is not the unique product entry')
    expected, actual = Path(binary).stat(), (proc / 'exe').stat()
    if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino):
        raise ValueError('actual service executable is not the prepared inode')
    service_cgroup.observe(record, pids=[pid])
    current = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
    if process.poll() is not None or current[0] == 'Z' or int(current[19]) != ticks:
        raise ValueError('service identity changed during observation')
    return dict(pid=pid, start_time_ticks=ticks, uid=uid, gid=gid,
                executable=dict(device=actual.st_dev, inode=actual.st_ino))


class Processes:
    """Exact owned children for ONE center/Redis and one or two host pools.

    Inputs are already staged/protected and reviewed by the enclosing system
    fixture. This object does not invent budgets, register templates, attest
    health or release VM responsibility. Restart always uses the same role
    configuration; a center failure cannot accidentally kill a healthy host.
    """
    def __init__(self, *, center, hosts, redis, record, logs):
        if (set(center) != {'binary', 'config', 'uid', 'gid'}
                or set(redis) != {'binary', 'config', 'uid', 'gid'}
                or not 1 <= len(hosts) <= 2
                or any(set(host) != {'binary', 'config', 'uid', 'gid'}
                       or type(host['uid']) is not int or type(host['gid']) is not int
                       or (host['uid'], host['gid']) != (0, 0) for host in hosts)
                or service_cgroup.service_identity(center['uid'], center['gid']) is None
                or service_cgroup.service_identity(redis['uid'], redis['gid']) is None
                or center['uid'] == redis['uid']
                or len({str(role['config']) for role in [center, redis, *hosts]}) != len(hosts) + 2):
            raise ValueError('one exact center, independent Redis and bounded private host cohort required')
        self.bindings = copy.deepcopy(dict(center=center, redis=redis,
            **{'host-' + str(index): host for index, host in enumerate(hosts)}))
        self.record, self.logs = Path(record), Path(logs)
        self.children = {}

    def _role(self, name):
        if name not in self.bindings:
            raise ValueError('exact registered process role required')
        return self.bindings[name]

    def start(self, name, *, env=None):
        role = self._role(name)
        faults = fault_environment(env)
        if faults and not name.startswith('host-'):
            raise ValueError('fault controls cannot affect center or Redis')
        previous = self.children.get(name)
        if previous is not None and previous.poll() is None:
            raise RuntimeError('original role is still running; no duplicate start')
        if name == 'redis':
            prefix = service_cgroup.prefix(self.record, uid=role['uid'], gid=role['gid'])
            with (self.logs / 'redis.log').open('ab') as output:
                process = subprocess.Popen([*prefix, str(role['binary']), str(role['config'])],
                    stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
                    env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, start_new_session=True)
        else:
            process = launch(role['binary'], role['config'], self.logs / (name + '.log'),
                             self.record, uid=role['uid'], gid=role['gid'], env=faults)
        self.children[name] = process
        return process

    def initialize(self, name):
        role = self._role(name)
        if name == 'redis' or (name in self.children and self.children[name].poll() is None):
            raise ValueError('explicit provisioning only before this product service starts')
        return command(role['binary'], role['config'], self.record,
                       uid=role['uid'], gid=role['gid'], action='initialize')

    def stop(self, name, *, force=False):
        self._role(name)
        process = self.children.get(name)
        if process is None or process.poll() is not None:
            return
        process.kill() if force else process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)

    def audit(self, name):
        role = self._role(name)
        if name == 'redis':
            raise ValueError('Redis is inspected by original center authority, not a raw audit path')
        if name == 'center' and name in self.children and self.children[name].poll() is None:
            raise RuntimeError('stop center before its original writer-lease audit')
        return command(role['binary'], role['config'], self.record, uid=role['uid'], gid=role['gid'])

    def observe(self):
        if set(self.children) != set(self.bindings):
            raise ValueError('every configured role must have an actual owned child')
        observed = {}
        for name, role in self.bindings.items():
            argv = [str(role['binary']), str(role['config'])] if name == 'redis' else None
            observed[name] = process_facts(self.children[name], role['binary'], role['config'],
                self.record, uid=role['uid'], gid=role['gid'], argv=argv)
        return observed

    def center_readiness(self, *, timeout):
        """Read live center state over each original host's pinned TLS source.

        No stop, audit, initialize, cache dispatch or local-store bypass.
        """
        import time
        deadline = time.monotonic() + timeout
        nodes, catalogs = [], {}
        for name, role in self.bindings.items():
            if not name.startswith('host-'):
                continue
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('bounded center readiness observation expired')
            view = command(role['binary'], role['config'], self.record,
                           action='observe-center', timeout=remaining)
            if (set(view) != {'schema_version','node','catalog'} or view['schema_version'] != 1
                    or type(view['node']) is not dict or type(view['catalog']) is not list):
                raise ValueError('invalid protected center readiness projection')
            node = view['node']
            identity = node['identity']['node_id']
            if identity in catalogs:
                raise ValueError('duplicate protected node observation')
            nodes.append(node)
            catalogs[identity] = view['catalog']
        return dict(nodes=nodes, catalogs=catalogs)

    def stop_all(self):
        errors = []
        for name in ('center', *reversed([name for name in self.bindings if name.startswith('host-')]), 'redis'):
            try:
                self.stop(name)
            except (OSError, subprocess.SubprocessError) as error:
                errors.append(type(error).__name__)
        if errors:
            raise RuntimeError('owned service child cleanup unknown; retain every original role directory')
