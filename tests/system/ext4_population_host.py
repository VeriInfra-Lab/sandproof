"""Opt-in single-host supervision of the real guest ext4 population probe.

The original complete two-pool fixture owns the center, Redis and private hosts.
Its protected backend credential authorizes the original private build window;
checked offline audits provide the exact global-to-native binding. This process
never mounts a host filesystem, changes networking, retries VM creation or
cleans host resources directly. Unknown retains the original fixture/evidence.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import sys
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
SYSTEM = Path(__file__).resolve().parent
for directory in (ROOT, SYSTEM):
    if str(directory) not in sys.path:
        sys.path.insert(0, str(directory))

from tools import check_kvm_host as host
from tools.template_build_config import host_uid_range, protected_json
from tools.template_physical_storage import envelope
from tools.template_build_service import Backend, configuration
from tools.template_build_store import BuildError
from tools.runtime_create_recovery import terminal_result
from _center_host_services import local_binding

GUEST_FILES = {
    'template_worker.py': ROOT / 'tools/template_worker.py',
    'template_inputs.py': ROOT / 'tools/template_inputs.py',
    'ext4_population_guest.py': SYSTEM / 'ext4_population_guest.py',
}
CASES = {'success', 'slow-unmount', 'cp', 'mount'}
VM_SECONDS = 600
GUEST_SECONDS = 300
CLEANUP_SECONDS = 120
MAX_JSON_BYTES = 65536


class RequestFailed(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__('controller request failed')


def digest(path):
    checksum = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            checksum.update(block)
    return checksum.hexdigest()


def private_json(path):
    path = host.trusted_path(Path(path))
    if path.stat().st_nlink != 1:
        raise ValueError('single-link private input required')
    return protected_json(path, maximum=1024 * 1024)


def write_new(path, value):
    """Only new private evidence; sync the directory before external work."""
    path = Path(path)
    parent = host.trusted_path(path.parent)
    if not parent.is_dir() or parent.stat().st_mode & 0o077:
        raise ValueError('root-private evidence parent required')
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
    with os.fdopen(os.open(path, flags, 0o600), 'w') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def validate_configuration(settings, center, build):
    """Use the original typed fixture; never upgrade an old public host config."""
    if {'bind', 'domain', 'credentials', 'internal_build_vms', 'build_retry_windows',
            'template_registry', 'request_admission'} & set(settings):
        raise ValueError('retired public host configuration is unsupported')
    bounded = settings.get('bounded_host', {})
    private = center.get('private_builds')
    boundary = center.get('private_build_boundary')
    if (not settings.get('maintenance', {}).get('capacity') or not private or not boundary
            or bounded.get('console_policy') != 'discard'
            or boundary['bind'] != build['center']['private_address']
            or build['center']['domain'] != center['domain']
            or not re.fullmatch(r'127\.0\.0\.1:[0-9]{1,5}', boundary['bind'])):
        raise ValueError('deployed complete private bounded builder required')
    uid_range = host_uid_range(settings)
    profile = bounded.get('physical_storage')
    envelope(profile)  # Reuse the existing physical-profile contract.
    pools = [pool for pool in profile['pools'] if pool['namespace'] == settings['namespace']]
    if (len(pools) != 1 or pools[0]['role'] != 'builder'
            or any(pools[0][field] != settings[field]
                   for field in ('database', 'node_state_dir', 'chroot_base'))):
        raise ValueError('physical profile must name the configured builder')
    total = settings.get('max_lifetime_secs')
    if total is None:
        total = settings.get('max_timeout_secs', 0)
    if min(settings.get('max_timeout_secs', 0), total) < VM_SECONDS:
        raise ValueError('unchanged builder lifetime must admit 600 seconds')
    policy = private['policy']
    if min(policy['max_timeout_secs'], policy['max_lifetime_secs']) < VM_SECONDS:
        raise ValueError('unchanged center builder lifetime must admit 600 seconds')
    return uid_range


def fixed_artifacts(settings, tenant):
    catalog = settings['templates']
    base = catalog['aliases'].get('base', 'base')
    matches = [row for row in catalog['templates'] if row['template_id'] == base]
    if len(matches) != 1 or matches[0]['owner'] not in (None, tenant):
        raise ValueError('authorized immutable builder base required')
    result = {}
    for name in ('kernel', 'rootfs'):
        spec = matches[0][name]
        if spec['path'] != settings[name] or Path(settings[name]).stat().st_size != spec['size_bytes']:
            raise ValueError('configured base differs from its immutable descriptor')
        observed = digest(host.trusted_path(Path(settings[name])))
        if observed != spec['sha256']:
            raise ValueError('fixed artifact fingerprint mismatch')
        result[name] = observed
    return result


class Client:
    """Thin use of the production private backend, not another HTTP algorithm."""
    def __init__(self, backend, owner):
        if type(owner) is not int or not 0 < owner < 2**64:
            raise ValueError('exact authenticated private owner required')
        self.backend, self.owner = backend, owner

    def __call__(self, method, path, body=None, *, timeout=60):
        try:
            return self.backend.request('builder', method, path, body,
                owner=self.owner, timeout=timeout)
        except BuildError as error:
            raise RequestFailed(error.code) from None


def sdk_sandbox(backend, owner, session):
    # Same fixed constructor used by the production trusted build backend.
    from e2b import Sandbox
    from e2b.connection_config import ConnectionConfig
    from packaging.version import Version
    center = backend.config['center']
    api_url = 'http://' + center['private_address']
    connection = ConnectionConfig(api_url=api_url, sandbox_url=api_url,
        domain=center['domain'], api_key=backend.backend_key, retries=0, request_timeout=30,
        extra_sandbox_headers={'X-Access-Token': session['envdAccessToken'],
            'x-sandproof-build-key': backend.backend_key, 'x-sandproof-build-owner': str(owner)})
    return Sandbox(sandbox_id=session['sandboxID'], sandbox_domain=center['domain'],
        envd_version=Version(session['envdVersion']), envd_access_token=session['envdAccessToken'],
        connection_config=connection)


def validate_guest(value):
    if (not isinstance(value, dict) or set(value) != {'passed', 'cases', 'entries', 'snapshot_sha256'}
            or value.get('passed') is not True
            or type(value.get('entries')) is not int or value['entries'] != 7
            or not re.fullmatch(r'[0-9a-f]{64}', value.get('snapshot_sha256', ''))):
        raise ValueError('guest evidence incomplete')
    cases = value.get('cases')
    if (not isinstance(cases, list) or len(cases) != len(CASES)
            or any(not isinstance(row, dict) or set(row) != {'case', 'passed', 'mount_and_loop_gone'}
                   or row.get('passed') is not True
                   or row.get('mount_and_loop_gone') is not True for row in cases)
            or {row.get('case') for row in cases} != CASES):
        raise ValueError('guest must pass exactly the four unchanged cases')
    return value


def sandbox_id(value):
    if not isinstance(value, str) or not re.fullmatch(
            r'sandbox-[1-9][0-9]*', value):
        raise ValueError('original sandbox identity invalid')
    return value


def mapped_context(center, observed, ident, *, node, owner):
    """Only the original checked authorities determine local IDs and ownership."""
    sandbox_id(ident)
    matches = [row for row in center['sandboxes']
               if row['create']['identity']['sandbox_id'] == ident]
    if (center['deployment'] != node['deployment_id'] or len(matches) != 1
            or matches[0]['private'] is not True):
        raise ValueError('original private center identity unknown or ambiguous')
    create = matches[0]['create']['identity']
    if create['node'] != node or create['tenant_id'] != str(owner) or create['intent_version'] != 1:
        raise ValueError('original private create binding mismatch')
    local = local_binding(observed, ident, expected_node=node)
    contexts = [context for context, _phase in observed['host_slots']
                if context['binding']['sandbox'] == local['sandbox']]
    if len(contexts) != 1 or contexts[0]['binding']['owner'] != local['owner']:
        raise ValueError('original sandbox lacks exact mapped host context')
    return create, local, contexts[0]


def capture_identity(settings, ident, center, observed, *, node, owner):
    from server_process_crash import process_start_ticks
    create, local, context = mapped_context(center, observed, ident, node=node, owner=owner)
    name = f"vm-{settings['namespace']}-{context['binding']['vm']}"
    claim = Path(settings['chroot_base']) / '.claims' / name
    result = dict(sandbox_id=ident, create_identity=create, local=local, context=context, resource_id=name,
        uid=settings['bounded_host']['uid_base'] + context['position'], process=None, network=None)
    record = claim / 'record.json'
    if record.exists():
        process = private_json(record)
        if (set(process) != {'resource_id', 'pid', 'start_time_ticks'}
                or process['resource_id'] != name
                or type(process['pid']) is not int or not 0 < process['pid'] < 2**32
                or type(process['start_time_ticks']) is not int or process['start_time_ticks'] <= 0):
            raise ValueError('claim identity mismatch')
        result['process'] = dict(resource_id=name, pid=process['pid'],
            start_time_ticks=process['start_time_ticks'], observed_alive=
            process_start_ticks(process['pid']) == process['start_time_ticks'])
    network = claim / 'network.json'
    named = Path('/run/netns') / name
    if network.exists() and named.exists():
        record = private_json(network)
        meta = named.stat()
        if record['resource_id'] != name or meta.st_ino != record['namespace_inode']:
            raise ValueError('network identity mismatch')
        result['network'] = [meta.st_dev, meta.st_ino]
    group = Path(settings['host_resources']['cgroup_parent']) / name
    if group.exists():
        record = private_json(claim / 'cgroup.json')
        meta = group.stat()
        if meta.st_ino != record['inode']:
            raise ValueError('cgroup identity mismatch')
        result['cgroup'] = [meta.st_dev, meta.st_ino]
    if result['process'] and result['process']['observed_alive']:
        process = Path('/proc') / str(result['process']['pid'])
        uid = int(next(line.split()[1] for line in (process / 'status').read_text().splitlines()
                       if line.startswith('Uid:')))
        if uid != result['uid'] or (process / 'cgroup').read_text().strip() != (
                '0::/' + str(group.relative_to('/sys/fs/cgroup'))):
            raise ValueError('original VMM UID/cgroup differs from its position')
        disk = (Path(settings['chroot_base']) / Path(settings['firecracker']).name / name / 'root/rootfs.ext4').stat()
        result['disk'] = [disk.st_dev, disk.st_ino, disk.st_size]
    return result


def namespace_holders(identity):
    """Compare nsfs device+inode, including FD-only relay/old-namespace holders."""
    if identity is None:
        return []
    found = []
    for process in Path('/proc').iterdir():
        if not process.name.isdecimal():
            continue
        try:
            paths = [process / 'ns/net', *list((process / 'fd').iterdir())]
            for path in paths:
                try:
                    meta = path.stat()
                    if [meta.st_dev, meta.st_ino] == identity:
                        found.append(int(process.name))
                        break
                except (FileNotFoundError, ProcessLookupError):
                    pass
        except (FileNotFoundError, ProcessLookupError):
            pass
    return sorted(found)


def controller_cleanup(center, observed, identity):
    """Read exact saved confirmations/frontiers, never infer Cleaned from absence."""
    terminal = terminal_result(center, observed, identity['create_identity'])
    if terminal is None:
        raise ValueError('original controller terminal confirmation unresolved')
    rows = [row for row in observed['native'] if row['sandbox'] == identity['local']['sandbox']]
    if rows and (len(rows) != 1 or rows[0]['owner'] != identity['local']['owner']
            or rows[0]['phase'] != 'Destroyed' or rows[0]['terminal'] is not True
            or rows[0]['current_vm'] is not None or any(rows[0]['resources'].values())):
        raise ValueError('original native controller state is not terminal')
    contexts = [(context, phase) for context, phase in observed['host_slots']
                if context['position'] == identity['context']['position']]
    if contexts != [(identity['context'], 'Free')]:
        raise ValueError('original checked host context is not Free')
    return {'original_state_terminal_or_retired': True, 'terminal_status': terminal['status']}


def complete_identity(identity):
    """Partial launch observations cannot stand in for historical identities."""
    return (isinstance(identity, dict) and bool(identity.get('process'))
            and bool(identity.get('network')) and bool(identity.get('cgroup')))


def audit_absent(settings, identity, center, observed):
    from sdk_c02_acceptance import processes_in_uid_range
    from server_process_crash import process_start_ticks
    if identity is not None and not complete_identity(identity):
        raise ValueError('original physical identity is incomplete')
    uids = host_uid_range(settings)
    if processes_in_uid_range(uids.start, uids.stop - 1):
        raise ValueError('builder UID process remains')
    namespace = settings['namespace']
    controller = {}
    if (any(phase != 'Free' for _context, phase in observed['host_slots'])
            or any(row['phase'] != 'Destroyed' or row['terminal'] is not True
                   or row['current_vm'] is not None or any(row['resources'].values())
                   for row in observed['native'])):
        raise ValueError('original checked builder authority is not empty or terminal')
    for path in (Path(settings['node_state_dir']) / 'host-slots').iterdir():
        slot = private_json(path)
        if slot.get('value') is not None and slot['value'].get('phase') != 'Free':
            raise ValueError('builder position is not confirmed Free')
    if identity is not None:
        controller = controller_cleanup(center, observed, identity)
        name = identity['resource_id']
        context = identity['context']
        slot = private_json(Path(settings['node_state_dir']) / 'host-slots' / (str(context['position']) + '.json'))
        if (slot.get('value', {}).get('context') != context
                or slot['value'].get('phase') != 'Free'):
            raise ValueError('original position is not confirmed Free')
        process = identity['process']
        if process and process_start_ticks(process['pid']) == process['start_time_ticks']:
            raise ValueError('original PID and start time remain')
        if namespace_holders(identity['network']):
            # Never label inode-number reuse alone a proven old holder. Such a
            # holder remains unconfirmed here and needs independent attribution.
            raise ValueError('original nsfs identity has a holder; attribution required')
        names = [name]
    else:
        names = []
    for parent in (Path(settings['chroot_base']) / Path(settings['firecracker']).name,
                   Path('/run/netns'), Path(settings['host_resources']['cgroup_parent'])):
        if parent.exists() and any(path.name.startswith(f'vm-{namespace}-') for path in parent.iterdir()):
            raise ValueError('builder namespace resource remains')
        if any((parent / name).exists() for name in names):
            raise ValueError('original host resource remains')
    return {'passed': True, 'original_position_free': identity is not None,
            'pid_start_absent': bool(identity and identity['process']), 'uid_pool_empty': True,
            'namespace_holders_absent': bool(identity and identity['network']), **controller}


class Observations:
    """Original owned planned audits; neither live-lease bypass nor attach."""
    def __init__(self, deployment, owner):
        self.deployment, self.owner = deployment, owner
        self.settings = deployment.settings[1]
        self.node = deployment.services.host_configs[1]['inbox']['node']

    def snapshots(self):
        # Each helper stops only its OWN original child and restarts the same
        # immutable config. Mixed observations stay unknown until exact pairs
        # agree; none is a cleanup permission or VM operation.
        observed = self.deployment.services.planned_audit('host-1')
        return self.deployment.center_snapshot(), observed

    def capture(self, ident):
        center, observed = self.snapshots()
        return capture_identity(self.settings, ident, center, observed,
                                node=self.node, owner=self.owner)

    def absent(self, identity):
        center, observed = self.snapshots()
        return audit_absent(self.settings, identity, center, observed)


def supervise(settings, request, make_sandbox, evidence, capture, audit, *,
              clock=time.monotonic, sleep=time.sleep, build_id=None):
    """One identity and one create POST; injected interfaces only support tests."""
    build_id = build_id or secrets.token_hex(16)  # valid_candidate_id: 32 lowercase hex.
    if not re.fullmatch(r'[0-9a-f]{32}', build_id):
        raise ValueError('invalid fixed build ID')
    report = dict(version=1, scope='single_host_ext4_population', build_id=build_id,
                  guest_passed=False, cleanup={'passed': False}, service_exit='pending_original_children')
    window, ident, identity = None, None, None
    deleted, capture_error, capture_attempts = False, None, 0

    def capture_original():
        nonlocal identity, capture_error, capture_attempts
        capture_attempts += 1
        try:
            candidate = capture(ident)
            if not complete_identity(candidate):
                # Preserve each observation without overwriting evidence or
                # freezing an intermediate launch state as the final identity.
                evidence('identity-partial-' + str(capture_attempts), candidate)
                raise ValueError('original physical identity is incomplete')
            evidence('identity', candidate)
            identity, capture_error = candidate, None  # Only after durable evidence.
            return True
        except Exception as error:
            capture_error = type(error).__name__
            return False

    evidence('intent', {'version': 1, 'build_id': build_id,
                        'deployment': settings['deployment'], 'owner': settings['owner']})
    try:
        opened = request('POST', '/sandproof/internal-build-windows', {})
        pattern = r'bw-' + re.escape(settings['deployment']) + '-' + str(settings['owner']) + r'-(0|[1-9][0-9]*)'
        if (not isinstance(opened, dict) or set(opened) != {'requestWindow', 'expiresAtMs'}
                or not isinstance(opened.get('requestWindow'), str)
                or not re.fullmatch(pattern, opened['requestWindow'])
                or type(opened.get('expiresAtMs')) is not int or opened['expiresAtMs'] <= 0):
            raise ValueError('private window response invalid')
        window = opened['requestWindow']
        evidence('window', opened)  # Durable before the single VM create.
        body = {'requestWindow': window, 'timeout': VM_SECONDS, 'allowInternetAccess': False}
        evidence('create', {'build_id': build_id, 'request': body})
        vm = request('POST', '/sandproof/internal-build-vms/' + build_id, body, timeout=VM_SECONDS)
        ident = sandbox_id(vm['sandboxID'])
        evidence('vm', {'sandboxID': ident, 'state': vm['state']})  # Never save the session token.
        if not capture_original() or not identity['process'].get('observed_alive'):
            raise ValueError('original live VMM identity incomplete')
        session = vm.get('session')
        if not isinstance(session, dict) or session.get('sandboxID') != ident:
            raise ValueError('original VM has no usable SDK session')
        sandbox = make_sandbox(session)
        result = sandbox.commands.run('mkdir -p /input /output', user='root', timeout=30)
        if result.exit_code != 0:
            raise ValueError('guest preparation failed')
        for name, source in GUEST_FILES.items():
            with source.open('rb') as stream:
                sandbox.files.write('/input/' + name, stream, user='root')
        result = sandbox.commands.run('python3 /input/ext4_population_guest.py', user='root', timeout=GUEST_SECONDS)
        if result.exit_code != 0 or len(result.stdout.encode()) > MAX_JSON_BYTES:
            raise ValueError('guest probe failed or exceeded its report bound')
        report['guest'] = validate_guest(json.loads(result.stdout, object_pairs_hook=host.unique_keys))
        report['guest_passed'] = True
    except BaseException as error:
        report['error_type'] = type(error).__name__  # No exception text/private SDK context.
    finally:
        try:
            if window is not None:
                closed = request('POST', '/sandproof/internal-build-windows/' + urllib.parse.quote(window, safe='') + '/close', {})
                if closed != {'requestWindow': window, 'closed': True}:
                    raise ValueError('original window closure unconfirmed')
                path = '/sandproof/internal-build-vms/' + build_id + '?' + urllib.parse.urlencode({'requestWindow': window})
                deadline = clock() + CLEANUP_SECONDS
                while True:
                    try:
                        vm = request('GET', path)
                    except RequestFailed as error:
                        if error.code != 404 or ident is not None:
                            raise
                        vm = None  # Only the preceding durable close permits this conclusion.
                    if vm is None:
                        break
                    current = sandbox_id(vm['sandboxID'])
                    if ident is not None and ident != current:
                        raise ValueError('original build identity changed')
                    ident = current
                    if identity is None:
                        capture_original()
                    if vm.get('cleanupComplete') is True:
                        break
                    if identity is None:
                        # Observe only the original accepted build while its
                        # claims can still be captured. A DELETE cannot restore
                        # identities that were never observed before retirement.
                        if clock() >= deadline:
                            raise TimeoutError('original physical identity unconfirmed')
                        sleep(0.2)
                        continue
                    if not deleted:
                        deleted = True
                        try:
                            request('DELETE', '/sandboxes/' + ident)
                        except Exception:
                            pass  # A refusal/unknown kill is not cleanup evidence.
                    if clock() >= deadline:
                        raise TimeoutError('original logical cleanup unconfirmed')
                    sleep(0.2)
                if capture_error or (ident is not None and not complete_identity(identity)):
                    # Logical cleanup does not justify unpinning an original
                    # VM whose historical physical identity is still unknown.
                    raise ValueError('original physical identity capture incomplete')
                settled = {'requestWindow': window, 'sandboxID': ident}
                evidence('settlement', settled)  # Before unpinning the private anchor.
                acknowledged = request('POST', '/sandproof/internal-build-vms/' + build_id + '/settle', settled)
                if acknowledged != {'requestWindow': window, 'settled': True}:
                    raise ValueError('private settlement unconfirmed')
                while True:
                    try:
                        report['cleanup'] = audit(identity)
                        break
                    except (ValueError, FileNotFoundError):
                        if clock() >= deadline:
                            raise TimeoutError('exact physical cleanup unconfirmed')
                        sleep(0.2)
            else:
                report['cleanup'] = audit(None)  # No VM create was dispatched.
        except BaseException as error:
            if ident is not None and not deleted:
                # Even a close/identity observation failure must attempt the
                # known original kill; it still cannot justify settlement.
                deleted = True
                try:
                    request('DELETE', '/sandboxes/' + ident)
                except Exception:
                    pass
            report['cleanup'] = {'passed': False, 'status': 'cleanup_unknown',
                                 'error_type': type(error).__name__}
    report['passed'] = report['guest_passed'] and report['cleanup'].get('passed') is True
    return report


def exercise(args):
    from sdk_c02_acceptance import processes_in_uid_range
    from _complete_registry_runtime import prepare_registry_runtime
    from server_process_crash import free_loopback_port
    from template_build import BUILDKIT_HASHES
    from tools.template_fetch import REGCTL_SHA256
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise ValueError('root isolated Linux host required')
    if importlib.metadata.version('e2b') != '2.51.0':
        raise ValueError('fixed SDK 2.51.0 required')
    revision = host.source_revision()
    limits, config_hashes = {}, {}
    for name in ('registry', 'build'):
        path = host.trusted_path(getattr(args, name + '_limits'))
        config_hashes[name] = digest(path)
        limits[name] = private_json(path)
        if digest(path) != config_hashes[name]:
            raise ValueError('approved limits changed during protected read')
    config_hashes['bounded_policy'] = digest(host.trusted_path(args.bounded_policy))
    tool_hashes = dict(BUILDKIT_HASHES, regctl=REGCTL_SHA256)
    if any(digest(host.trusted_path(args.tools / name)) != checksum for name, checksum in tool_hashes.items()):
        raise ValueError('fixed original builder tool fingerprint mismatch')
    inputs = {name: digest(path) for name, path in GUEST_FILES.items()}
    report_path = Path(args.report)
    parent = host.trusted_path(report_path.parent)
    if not parent.is_dir() or parent.stat().st_mode & 0o077 or report_path.exists():
        raise ValueError('new root-private report required')
    run_root = Path(args.run_root).absolute()
    if run_root == parent or run_root in parent.parents:
        raise ValueError('evidence must be outside managed allocation roots')
    evidence = lambda name, value: write_new(report_path.with_suffix('.' + name + '.json'), value)
    # Reserve the final path before issuing even a request window.
    fd = os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600)
    report = {'version': 1, 'scope': 'single_host_ext4_population', 'passed': False}
    deployment = None
    try:
        deployment = prepare_registry_runtime(args, tool_hashes=tool_hashes,
            registry_limits=limits['registry'], build_limits=limits['build'],
            ports=[free_loopback_port() for _ in range(3)])
        services, settings = deployment.services, deployment.settings[1]
        build = configuration(deployment.build_config, require_physical_lease=False)
        validate_configuration(settings, services.center_config, build)
        hashes = fixed_artifacts(settings, 1)
        if any(processes_in_uid_range(pool.start, pool.stop - 1)
               for pool in map(host_uid_range, deployment.settings)):
            raise ValueError('exclusive runtime or builder UID pool occupied')
        backend, observations = Backend(build), Observations(deployment, 1)
        services.start()
        services.wait_ready()
        manifest, processes = services.execution(secrets.token_hex(16), args.bounded_policy)
        evidence('execution', {'manifest': manifest, 'actual_processes': processes})
        observations.absent(None)
        original = dict(deployment=services.center_config['deployment_id'], owner=1)
        report = supervise(original, Client(backend, 1), lambda session: sdk_sandbox(backend, 1, session),
            evidence, observations.capture, observations.absent)
        report.update(source_commit=revision['revision'], config_sha256=config_hashes,
            candidate_manifest_sha256=services.manifest_sha256,
            execution_manifest_sha256=hashlib.sha256(json.dumps(manifest,
                sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
            host_kernel=os.uname().release, sdk='2.51.0', input_sha256=inputs,
            inputs_unchanged=inputs == {name: digest(path) for name, path in GUEST_FILES.items()},
            artifacts_before=hashes, artifacts_after={name: digest(settings[name]) for name in hashes},
            config_unchanged=all(digest(getattr(args, name + '_limits')) == config_hashes[name]
                for name in limits) and digest(args.bounded_policy) == config_hashes['bounded_policy'])
        report['artifacts_unchanged'] = report['artifacts_before'] == report['artifacts_after']
        report['passed'] &= report['artifacts_unchanged'] and report['config_unchanged'] and report['inputs_unchanged']
    except BaseException as error:
        report.update(passed=False, error_type=type(error).__name__)
    finally:
        if deployment is not None:
            try:
                deployment.services.stop_all()
                report['service_exit'] = 'confirmed_original_children'
            except BaseException as error:
                report.update(passed=False, service_exit='unknown', service_exit_error=type(error).__name__)
        # No cgroup/staging/resource deletion: an unknown original request must
        # retain its identity, anchor, leaf and private evidence for the owner.
        with os.fdopen(fd, 'w') as output:
            json.dump(report, output, sort_keys=True)
            output.flush()
            os.fsync(output.fileno())
    return report


def arguments():
    from _complete_registry_runtime import add_registry_arguments
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent',
                 'cgroup-parent', 'tools', 'registry-limits', 'build-limits'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--uid-base', required=True, type=int)
    add_registry_arguments(parser)
    parser.add_argument('--report', required=True, type=Path)
    return parser


if __name__ == '__main__':
    args = arguments().parse_args()
    try:
        result = exercise(args)
    except Exception as error:
        print(json.dumps({'passed': False, 'error_type': type(error).__name__}))
        sys.exit(1)
    print(json.dumps({'passed': result['passed'], 'scope': 'single_host_ext4_population',
                      'guest_passed': result.get('guest_passed', False),
                      'cleanup_passed': result.get('cleanup', {}).get('passed', False)}))
    sys.exit(0 if result['passed'] else 1)
