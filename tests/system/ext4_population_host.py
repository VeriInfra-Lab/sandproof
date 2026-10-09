"""Opt-in single-host supervision of the real guest ext4 population probe.

The operator starts an approved server with its existing complete private
builder configuration and remains responsible for server/service-group exit.
This process never starts a server, mounts a host filesystem, changes networking,
retries VM creation, or cleans host resources directly. All evidence is retained.
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
import importlib.metadata
import json
import os
from pathlib import Path
import re
import secrets
import sqlite3
import stat
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
SYSTEM = Path(__file__).resolve().parent
for directory in (ROOT, SYSTEM):
    if str(directory) not in sys.path:
        sys.path.insert(0, str(directory))

from tools import check_kvm_host as host
from tools.template_build_config import host_uid_range, protected_json
from tools.template_physical_storage import envelope

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


def validate_configuration(settings, api_url):
    if api_url != 'http://' + settings.get('bind', '') or not re.fullmatch(
            r'http://127\.0\.0\.1:[0-9]{1,5}', api_url):
        raise ValueError('exact configured loopback builder URL required')
    bounded = settings.get('bounded_host', {})
    if (settings.get('internal_build_vms') is not True or settings.get('template_registry')
            or not settings.get('maintenance', {}).get('capacity')
            or not settings.get('build_retry_windows') or bounded.get('console_policy') != 'discard'):
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
    credentials = settings.get('credentials')
    if (not isinstance(credentials, list) or len(credentials) != 1
            or type(credentials[0].get('tenant')) is not int or credentials[0]['tenant'] <= 0
            or not isinstance(credentials[0].get('secret'), str) or not credentials[0]['secret']):
        raise ValueError('one independent private builder credential required')
    return credentials[0], uid_range


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
    def __init__(self, api_url, key):
        self.api_url, self.key = api_url, key
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def __call__(self, method, path, body=None, *, timeout=60):
        request = urllib.request.Request(self.api_url + path, method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={'X-API-Key': self.key, 'Content-Type': 'application/json'})
        try:
            with self.opener.open(request, timeout=timeout) as response:
                raw = response.read(MAX_JSON_BYTES + 1)
                if not 200 <= response.status < 300:
                    raise RequestFailed(response.status)
        except urllib.error.HTTPError as error:
            raise RequestFailed(error.code) from None
        if len(raw) > MAX_JSON_BYTES:
            raise ValueError('bounded controller response exceeded')
        return json.loads(raw, object_pairs_hook=host.unique_keys) if raw else None


def sdk_sandbox(settings, api_url, key, session):
    # Same fixed constructor used by the production trusted build backend.
    from e2b import Sandbox
    from e2b.connection_config import ConnectionConfig
    from packaging.version import Version
    connection = ConnectionConfig(api_url=api_url, sandbox_url=api_url,
        domain=settings['domain'], api_key=key, retries=0, request_timeout=30,
        extra_sandbox_headers={'X-Access-Token': session['envdAccessToken']})
    return Sandbox(sandbox_id=session['sandboxID'], sandbox_domain=settings['domain'],
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


def sandbox_id(value, namespace):
    if not isinstance(value, str) or not re.fullmatch(
            r'sb-' + re.escape(namespace) + r'-[1-9][0-9]*', value):
        raise ValueError('original sandbox identity invalid')
    return value


def capture_identity(settings, ident):
    from server_process_crash import process_start_ticks
    ident = sandbox_id(ident, settings['namespace'])
    number = int(ident.rsplit('-', 1)[1])
    with closing(sqlite3.connect(Path(settings['database']).as_uri() + '?mode=ro', uri=True, timeout=2)) as db:
        contexts = [json.loads(row[0]) for row in db.execute(
            'SELECT context_json FROM controller_host_slots')]
    contexts = [row for row in contexts if row['binding']['sandbox'] == number]
    if len(contexts) != 1:
        raise ValueError('original sandbox lacks unique host context')
    context = contexts[0]
    if context['binding']['owner'] != settings['credentials'][0]['tenant']:
        raise ValueError('original host context belongs to another owner')
    name = f"vm-{settings['namespace']}-{context['binding']['vm']}"
    claim = Path(settings['chroot_base']) / '.claims' / name
    result = dict(sandbox_id=ident, context=context, resource_id=name,
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


def controller_cleanup(settings, identity):
    """Short WAL-aware SQL read; never hold an external reader across maintenance."""
    with closing(sqlite3.connect(Path(settings['database']).as_uri() + '?mode=ro', uri=True, timeout=2)) as db:
        ident = identity['sandbox_id']
        row = db.execute('SELECT state_blob FROM sandboxes WHERE sandbox_id=?', (ident,)).fetchone()
        if row is not None:
            state = json.loads(row[0])
            if (state['phase'] != 'Destroyed' or state['terminal'] is not True
                    or state['current_vm'] is not None or state['net_slot'] is not None):
                raise ValueError('original controller state is not terminal')
        pending = db.execute("SELECT count(*) FROM outbox WHERE sandbox_id=? AND dispatch_state!='receipted'", (ident,)).fetchone()[0]
        anchors = db.execute('SELECT count(*) FROM private_build_anchors WHERE sandbox_id=?', (ident,)).fetchone()[0]
        row = db.execute('SELECT context_json FROM controller_host_slots WHERE position=?',
                         (identity['context']['position'],)).fetchone()
        if pending or anchors or row is None or json.loads(row[0]) != identity['context']:
            raise ValueError('original controller work, anchor or context unresolved')
    return {'original_state_terminal_or_retired': True, 'pending_actions': 0, 'private_anchors': 0}


def complete_identity(identity):
    """Partial launch observations cannot stand in for historical identities."""
    return (isinstance(identity, dict) and bool(identity.get('process'))
            and bool(identity.get('network')) and bool(identity.get('cgroup')))


def audit_absent(settings, identity):
    from sdk_c02_acceptance import processes_in_uid_range
    from server_process_crash import process_start_ticks
    if identity is not None and not complete_identity(identity):
        raise ValueError('original physical identity is incomplete')
    uids = host_uid_range(settings)
    if processes_in_uid_range(uids.start, uids.stop - 1):
        raise ValueError('builder UID process remains')
    namespace = settings['namespace']
    controller = {}
    for path in (Path(settings['node_state_dir']) / 'host-slots').iterdir():
        slot = private_json(path)
        if slot.get('value') is not None and slot['value'].get('phase') != 'Free':
            raise ValueError('builder position is not confirmed Free')
    if identity is not None:
        controller = controller_cleanup(settings, identity)
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


def supervise(settings, request, make_sandbox, evidence, capture, audit, *,
              clock=time.monotonic, sleep=time.sleep, build_id=None):
    """One identity and one create POST; injected interfaces only support tests."""
    build_id = build_id or secrets.token_hex(16)  # valid_candidate_id: 32 lowercase hex.
    if not re.fullmatch(r'[0-9a-f]{32}', build_id):
        raise ValueError('invalid fixed build ID')
    report = dict(version=1, scope='single_host_ext4_population', build_id=build_id,
                  guest_passed=False, cleanup={'passed': False}, server_exit='outer_supervisor')
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

    evidence('intent', {'version': 1, 'build_id': build_id, 'namespace': settings['namespace']})
    try:
        opened = request('POST', '/sandproof/internal-build-windows', {})
        tenant = settings['credentials'][0]['tenant']
        pattern = f"bw-{settings['namespace']}-{tenant}-(0|[1-9][0-9]*)"
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
        ident = sandbox_id(vm['sandboxID'], settings['namespace'])
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
                    current = sandbox_id(vm['sandboxID'], settings['namespace'])
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
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise ValueError('root isolated Linux host required')
    if importlib.metadata.version('e2b') != '2.51.0':
        raise ValueError('fixed SDK 2.51.0 required')
    config_sha = digest(host.trusted_path(args.config))
    settings = private_json(args.config)
    if digest(args.config) != config_sha:
        raise ValueError('configuration changed during its protected read')
    credential, uids = validate_configuration(settings, args.api_url)
    revision = host.source_revision()
    hashes = fixed_artifacts(settings, credential['tenant'])
    inputs = {name: digest(path) for name, path in GUEST_FILES.items()}
    report_path = Path(args.report)
    parent = host.trusted_path(report_path.parent)
    if not parent.is_dir() or parent.stat().st_mode & 0o077 or report_path.exists():
        raise ValueError('new root-private report required')
    managed = [Path(settings['database']).parent, Path(settings['node_state_dir']), Path(settings['chroot_base'])]
    public = settings['bounded_host']['physical_storage']['public_build']
    if public:
        managed += [Path(public['root']), Path(public['registry_root'])]
    if any(parent == root or root in parent.parents for root in managed):
        raise ValueError('evidence must be outside managed allocation roots')
    evidence = lambda name, value: write_new(report_path.with_suffix('.' + name + '.json'), value)
    # Reserve the final path before issuing even a request window.
    fd = os.open(report_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600)
    report = {'version': 1, 'scope': 'single_host_ext4_population', 'passed': False}
    try:
        if processes_in_uid_range(uids.start, uids.stop - 1):
            raise ValueError('exclusive builder UID pool occupied')
        audit_absent(settings, None)
        report = supervise(settings, Client(args.api_url, credential['secret']),
            lambda session: sdk_sandbox(settings, args.api_url, credential['secret'], session),
            evidence, lambda ident: capture_identity(settings, ident),
            lambda identity: audit_absent(settings, identity))
        report.update(source_commit=revision['revision'], config_sha256=config_sha,
            host_kernel=os.uname().release, sdk='2.51.0', input_sha256=inputs,
            inputs_unchanged=inputs == {name: digest(path) for name, path in GUEST_FILES.items()},
            artifacts_before=hashes, artifacts_after={name: digest(settings[name]) for name in hashes},
            config_unchanged=digest(args.config) == config_sha)
        report['artifacts_unchanged'] = report['artifacts_before'] == report['artifacts_after']
        report['passed'] &= report['artifacts_unchanged'] and report['config_unchanged'] and report['inputs_unchanged']
    except BaseException as error:
        report.update(passed=False, error_type=type(error).__name__)
    finally:
        with os.fdopen(fd, 'w') as output:
            json.dump(report, output, sort_keys=True)
            output.flush()
            os.fsync(output.fileno())
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--api-url', required=True)
    parser.add_argument('--config', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    try:
        result = exercise(args)
    except Exception as error:
        print(json.dumps({'passed': False, 'error_type': type(error).__name__}))
        sys.exit(1)
    print(json.dumps({'passed': result['passed'], 'scope': 'single_host_ext4_population',
                      'guest_passed': result.get('guest_passed', False),
                      'cleanup_passed': result.get('cleanup', {}).get('passed', False)}))
    sys.exit(0 if result['passed'] else 1)
