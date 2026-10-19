"""Trusted Rust envd acceptance policy and receipts; never a product default.

The existing KVM supervisor owns admission, launch intent and recovery. This
module supplies only the fixed suite and checked candidate/result boundary.
"""
from pathlib import Path
import math

from tools import guest_artifacts
from tools.check_kvm_host import require_keys
from tools.system_acceptance import digest, json_file, private_path, write_once

FILES = ('configuration', 'policy_template', 'oci_archive', 'go_rootfs', 'builder_rootfs', 'budget_profile')
SUITE = ('paired', 'input', 'input-chunks', 'pty', 'command-deadlines',
         'rpc-deadlines', 'file-deadlines', 'file-permissions', 'faults')
PROBES = dict(paired='sdk_guest_compare.py', input='sdk_command_input.py',
    **{'input-chunks':'sdk_guest_input_limits.py', 'pty':'sdk_pty.py',
       'command-deadlines':'sdk_command_timeouts.py', 'rpc-deadlines':'sdk_rpc_timeouts.py',
       'file-deadlines':'sdk_file_timeouts.py', 'file-permissions':'sdk_file_permissions.py',
       'faults':'sdk_guest_faults.py'})
FAULTS = ('lost-start', 'restart-old-pid', 'full-filesystem', 'slow-output',
          'registry-reclaim', 'lifecycle', 'unavailable-expiry')


def policy(config):
    value = config.get('rust_envd_acceptance')
    if value is None:
        return None  # Existing Go gate remains unchanged; no Rust PASS implied.
    require_keys(value, {*FILES, 'tls_material', 'registry_limits', 'build_limits', 'data_transfer'}, 'Rust envd acceptance')
    for name in FILES:
        path = private_path(value[name])
        expected = config['host_assets'].get(str(path))
        if expected is None or digest(path) != expected:
            raise ValueError('unapproved or changed envd acceptance input')
    # The serial Rust stage has its own reviewed original service envelope.
    # Never enlarge the Go/DK/Harbor budget or inherit its one-CPU limit for a
    # private builder whose original resource shape requires two vCPUs.
    host = json_file(private_path(value['budget_profile'])).get('host')
    if type(host) is not dict or type(host.get('control_cpu_allowance')) is not int:
        raise ValueError('explicit Rust stage service budget required')
    from tools.service_cgroup import limits
    limits(host['control_cpu_allowance'] * 100000, host.get('control_memory_allowance_bytes'))
    # The existing prepared two-task gate has only one host certificate. This
    # stage builds privately on a second host role; never rotate the old gate's
    # material or reuse one node's leaf for both roles just to make it start.
    from tools.ci_e2e_artifacts import tls_material_files
    tls=private_path(value['tls_material'],directory=True)
    for name in tls_material_files(('runtime','builder')):
        path=private_path(tls/name)
        expected=config['host_assets'].get(str(path))
        if not 0<path.stat().st_size<=64*1024 or expected is None or digest(path)!=expected:
            raise ValueError('complete approved stage-specific TLS material required')
    from tools.template_build_config import bounded_data_route_limits
    bounded_data_route_limits({'data_transfer': value['data_transfer']})
    return value


def prepared_paths(config):
    """Stable per-intent outputs, not a mutable default template/profile."""
    root = Path(config['candidate_manifest']).with_suffix('')
    return dict(release=str(root) + '-guest', configuration=str(root) + '-guest-config.json',
                profile=str(root) + '-guest-profile.json')


def prepare(config):
    value = policy(config)
    if value is None:
        return
    from tools.prepare_guest import prepare as build_guest
    paths = prepared_paths(config)
    result = build_guest(dict(candidate=config['candidate'], checkout=config['checkout'],
        output=paths['release'], cargo_home=config['cargo_home'], toolchain_bin=config['toolchain_bin'],
        timeout_seconds=900, max_log_bytes=2*1024**2))
    # Only the newly built worker identity changes. All approved numerical
    # budgets, paths and schema remain untouched; check() still owns validity.
    configuration = json_file(private_path(value['configuration']))
    manifest = guest_artifacts.load(paths['release'], result['manifest_sha256'])
    if configuration.get('version') != manifest['versions']['config_version']:
        raise ValueError('approved guest configuration version differs from candidate')
    configuration['worker'] = result['worker']
    write_once(Path(paths['configuration']), configuration)
    profile = dict(release=paths['release'], manifest_sha256=result['manifest_sha256'],
        configuration=paths['configuration'], configuration_sha256=digest(Path(paths['configuration'])))
    write_once(Path(paths['profile']), profile)
    return profile


def identity(config):
    from tools.template_guest import load_profile
    paths = prepared_paths(config)
    profile, expected = load_profile(paths['profile'])
    manifest = guest_artifacts.load(profile['release'], profile['manifest_sha256'])
    if manifest['source']['commit'] != config['candidate']:
        raise ValueError('Rust envd artifact is not the supervised candidate')
    return expected


def require_cleanup(report, *, candidate, run_id, expected_guest):
    """Physical recovery is independent of whether the checks passed."""
    if (type(report) is not dict or report.get('candidate') != candidate
            or report.get('run_id') != run_id
            or report.get('guest') != expected_guest
            or report.get('resource_cleanup_confirmed') is not True
            or report.get('staging_cleanup_confirmed') is not True
            or report.get('service_cleanup_confirmed') is not True):
        raise ValueError('Rust envd candidate or exact cleanup unconfirmed')
    return dict(candidate=candidate, run_id=run_id, guest=expected_guest,
                resource_cleanup_confirmed=True, staging_cleanup_confirmed=True,
                service_cleanup_confirmed=True)


def require_result(report, *, candidate, run_id, expected_guest):
    """A zero child exit or a Go/partial suite can never satisfy this gate."""
    require_cleanup(report,candidate=candidate,run_id=run_id,expected_guest=expected_guest)
    if report.get('status') != 'pass':
        raise ValueError('Rust envd checks did not pass')
    checks = report.get('checks')
    if (type(checks) is not list or any(type(row) is not dict for row in checks)
            or [row.get('name') for row in checks] != list(SUITE)
            or any(type(row.get('returncode')) is not int or row['returncode'] != 0 for row in checks)):
        raise ValueError('complete original Rust envd suite did not pass')
    faults = report.get('faults')
    if (type(faults) is not list or any(type(row) is not dict for row in faults)
            or [row.get('name') for row in faults] != list(FAULTS)
            or any(row.get('status') != 'pass' for row in faults)):
        raise ValueError('mandatory envd fault evidence incomplete')
    image = report.get('immutable_template')
    if type(image) is not dict or image.get('unchanged') is not True:
        raise ValueError('original shared Rust rootfs observation missing')
    guest_artifacts.descriptor({key:image.get(key) for key in ('bytes','sha256')}, 2**64-1)
    environment = report.get('environment')
    environment_keys = ('installed','system_file','fresh_control_pristine','daemon_unchanged','host_system_path_absent')
    if type(environment) is not dict or any(environment.get(key) is not True for key in environment_keys):
        raise ValueError('S03 dependency/system isolation evidence incomplete')
    clean_checks = []
    for row in checks:
        seconds = row.get('seconds')
        if type(seconds) not in (int,float) or not math.isfinite(seconds) or not 0 <= seconds <= 900:
            raise ValueError('finite original probe duration required')
        clean_checks.append(dict(name=row['name'],returncode=0,seconds=seconds))
    return dict(candidate=candidate, run_id=run_id, status='pass', guest=expected_guest,
                checks=clean_checks, faults=[dict(name=row['name'],status='pass') for row in faults], resource_cleanup_confirmed=True,
                staging_cleanup_confirmed=True,
                service_cleanup_confirmed=True,
                immutable_template={key:image[key] for key in ('bytes','sha256','unchanged')},
                environment={key:True for key in environment_keys})
