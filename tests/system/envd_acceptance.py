"""Same-candidate Rust envd stage for the existing trusted KVM supervisor.

Builds a new immutable test template, not the production default. The original
center/host fixture, durable audits, cgroup owner and SDK probes do the work.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
from tools.capture_acceptance import original_chroot
import secrets
import signal
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools import ci_e2e_artifacts, envd_acceptance, service_cgroup
from tools.benchmark_ci import instantiate_policy
from tools.system_acceptance import digest, json_file, private_path, write_once
from tools.template_worker import Commands
from tools.template_guest import load_profile
from _complete_registry_runtime import prepare_registry_runtime, assert_terminal_rows, _save
from _envd_template import build as build_template
from _runtime_services import RuntimeServices
from sdk_c02_acceptance import processes_in_uid_range
from sdk_guest_environment import cached_template, MARKER
from server_process_crash import free_loopback_port, fetch
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES
from tools.template_fetch import REGCTL_SHA256

GROUP_SECONDS = 3600


def checked_config(path):
    config = json_file(private_path(path))
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise ValueError('isolated root Linux/KVM required')
    import importlib.metadata
    if importlib.metadata.version('e2b') != '2.51.0':
        raise ValueError('fixed offline SDK required')
    if subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip() != config['candidate']:
        raise ValueError('runner is not the supervised exact candidate')
    if subprocess.check_output(['git', '-C', str(ROOT), 'status', '--porcelain']):
        raise ValueError('clean supervised checkout required')
    envd_acceptance.policy(config)
    expected = envd_acceptance.identity(config)
    parent = private_path(config['scratch_parent'], directory=True)
    root = Path(config['run_root'])
    if root.parent != parent or root.name in ('', '.', '..'):
        raise ValueError('new directly owned acceptance root required')
    if processes_in_uid_range(config['uid_base'], config['uid_base'] + 164):
        raise ValueError('exclusive test UID range occupied')
    return config, expected


def no_resources(deployment):
    """Read original durable terminal states AND independent physical absence."""
    deadline = time.monotonic() + 60
    while True:
        try:
            for index, native in enumerate(deployment.settings):
                root = deployment.root / str(index)
                if (original_chroot(root) / '.claims').is_dir():
                    check_no_resources(root, deployment.namespaces[index], Path(native['firecracker']),
                                       native['uid_base'], native=native)
                assert_terminal_rows(root / 'db/controller.db')
            assert not processes_in_uid_range(deployment.settings[0]['uid_base'],
                                             deployment.settings[1]['uid_base'] + 2)
            return
        except AssertionError:
            assert time.monotonic() < deadline, 'original resource cleanup remains unknown'
            time.sleep(.2)


def prepare_deployment(config, root, record):
    from types import SimpleNamespace
    value = config['rust_envd_acceptance']
    args = SimpleNamespace(**{name:Path(config[name]) for name in (
        'center', 'host', 'candidate_manifest', 'redis_material', 'service_staging_parent',
        'kernel', 'firecracker', 'jailer', 'tools', 'cgroup_parent', 'build_cgroup_parent')})
    for name in ('center_uid', 'center_gid', 'redis_uid', 'redis_gid', 'uid_base'):
        setattr(args, name, config[name])
    args.tls_material=Path(value['tls_material'])
    args.rootfs, args.builder_rootfs = Path(value['go_rootfs']), Path(value['builder_rootfs'])
    args.run_root, args.scratch_parent = root / 'runtime', root
    args.namespace, args.build_namespace = [secrets.token_hex(8) for _ in range(2)]
    args.service_cgroup_record = record
    policy = instantiate_policy(json_file(Path(value['policy_template'])), args.run_root,
                                [args.namespace, args.build_namespace], job_count=4)
    helper = policy['public']['bounded_builds']['helper']
    helper.update(path=config['window_helper'], sha256=digest(Path(config['window_helper'])))
    args.bounded_policy = root / 'policy.json'
    write_once(args.bounded_policy, policy)
    return prepare_registry_runtime(args, tool_hashes={k:v for k,v in BUILDKIT_HASHES.items() if k != 'envd'} | {'regctl':REGCTL_SHA256},
        registry_limits=value['registry_limits'], build_limits=value['build_limits'], runtime_shape=(1,512),
        ports=[free_loopback_port() for _ in range(3)], data_transfer=value['data_transfer'],
        activation_ceiling_bytes=200*1024**3,
        rust_guest_profile=envd_acceptance.prepared_paths(config)['profile']), args


def observe_probe_failure(deployment, root, name, deadline, primary):
    """One read-only diagnostic before recovery, within the original deadline.

    This is a later observation, never evidence of the exact failing request's
    state or permission to replay it. No service stop/restart or audit occurs.
    """
    started = time.monotonic()
    report = dict(probe=name, probe_failure=type(primary).__name__,
                  started_unix_ms=time.time_ns()//1_000_000,
                  scope='after probe exit, before recovery; not request-time state or a readiness verdict')
    remaining = min(5, deadline-started)
    if remaining <= 0:
        report['center_failure'] = 'probe_deadline_elapsed'
    else:
        try:
            report['center'] = deployment.services.processes.center_readiness(timeout=remaining)
        except BaseException as error:
            report['center_failure'] = type(error).__name__[:128]
    try:
        report['cgroup'] = service_cgroup.observe(root/'service-cgroup.json')
    except BaseException as error:
        report['cgroup_failure'] = type(error).__name__[:128]
    report.update(finished_unix_ms=time.time_ns()//1_000_000,
                  elapsed_seconds=round(time.monotonic()-started, 3))
    try:
        write_once(root/(name+'-failure.json'), report)
    except BaseException as error:
        raise BaseExceptionGroup('probe and diagnostic persistence failed', [primary, error])


def run_probe(deployment, config, root, name, template):
    """Run the original cases, not another implementation of their assertions."""
    from tools.envd_acceptance import PROBES, prepared_paths
    started = time.monotonic()
    deployment.services.wait_ready()
    pins, guest = load_profile(prepared_paths(config)['profile'])
    arguments = [sys.executable, str(ROOT/'tests/system'/PROBES[name]),
        '--api-url', deployment.services.sdk_url, '--sandbox-url', deployment.services.sdk_url,
        '--template',template,'--guest-release',pins['release'],
        '--guest-manifest-sha256',pins['manifest_sha256'],
        '--guest-configuration',pins['configuration'],
        '--guest-configuration-sha256',pins['configuration_sha256']]
    if name == 'paired':
        arguments.extend(['--go-template','runtime-base','--observe-resources'])
    if name == 'faults':
        from _guest_restart_fixture import prepare
        fixture = prepare(deployment, root, template, guest)
        arguments.extend(['--restart-fixture', str(fixture)])
    if name == 'file-permissions':
        canary = root / 'host-canary'
        if not canary.exists():
            with canary.open('xb') as output:
                output.write(b'private-harness-canary\n')
        arguments.extend(['--host-canary',str(canary)])
    with (root/(name+'.log')).open('xb') as output:
        owner = Commands(started+900, output, 16*1024**2)
        try:
            owner.run(arguments, env={'PATH':'/usr/local/bin:/usr/bin:/bin', 'LANG':'C.UTF-8',
                'SANDPROOF_TEST_API_KEY':deployment.key, 'NO_PROXY':'*','no_proxy':'*'})
        except BaseException as error:
            observe_probe_failure(deployment, root, name, started+900, error)
            raise
    # Commands.run rejects failed exit/deadline/output: never manufacture code0.
    result = dict(name=name,returncode=0,seconds=round(time.monotonic()-started,3))
    no_resources(deployment)
    if name == 'file-permissions':
        assert (root/'host-canary').read_bytes() == b'private-harness-canary\n'
    return result


def stop_services(deployment, report):
    """Original role cleanup is allowed only after independent VM absence."""
    services = deployment.services
    services.stop_all()
    report['storage_peaks'] = services.storage_peaks()
    if report.get('resource_cleanup_confirmed') is True:
        services.cleanup_staging()
        report['staging_cleanup_confirmed'] = True


def reclaim_resources(deployment, report, *, services_started):
    # No VM request is issued before start completes. A partially initialized
    # authority is NOT permission to initialize again or wait for its listener.
    # Stop the exact children in the caller, retain role inputs/identity, and
    # let the original supervisor block for operator inspection.
    if services_started is not True:
        return
    services = deployment.services
    for role in ('host-0','host-1','center'):
        child=services.processes.children.get(role)
        if child is not None and child.poll() is not None:
            services.processes.start(role)
    services.wait_ready(for_admission=False)
    deployment.close_builders()
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    for row in deployment.center_snapshot()['sandboxes']:
        if row['private'] is False and row['released'] is False:
            code,_=fetch(opener,services.sdk_url,'DELETE',
                '/sandboxes/'+row['create']['binding']['session'],deployment.key)
            assert code in (200,204,404)
    no_resources(deployment)
    report['resource_cleanup_confirmed']=True


def finish_worker(deployment, report, root, *, services_started, primary):
    """Persist original case/cleanup evidence even when final observation fails."""
    errors = []
    if deployment is not None:
        for phase, operation in (
                ('resources', lambda: reclaim_resources(deployment, report, services_started=services_started)),
                ('services', lambda: stop_services(deployment, report))):
            try:
                operation()
            except BaseException as error:
                errors.append(error)
                report.setdefault('cleanup_failures', []).append(
                    dict(phase=phase, error_type=type(error).__name__[:128]))
        if errors:
            try:
                report['storage_diagnostics'] = deployment.services.storage_diagnostics()
            except BaseException as error:
                errors.append(error)
                report.setdefault('cleanup_failures', []).append(
                    dict(phase='storage-diagnostics', error_type=type(error).__name__[:128]))
    if primary is not None or errors:
        report['status'] = 'failed'
    try:
        _save(root/'report.json', report)
    except BaseException as error:
        errors.append(error)
    if errors:
        raise BaseExceptionGroup('envd stage or original cleanup failed',
                                 ([primary] if primary is not None else []) + errors)


def worker(config, expected):
    root = Path(config['run_root'])
    record = root / 'service-cgroup.json'
    service_cgroup.observe(record, pids=[os.getpid()])
    report = dict(candidate=config['candidate'],run_id=config['run_id'],guest=expected,
                  status='failed',checks=[],resource_cleanup_confirmed=False,
                  staging_cleanup_confirmed=False,service_cleanup_confirmed=False)
    deployment = None
    services_started = False
    primary = None
    try:
        report['phase']='preparing-deployment'
        _save(root/'report.json',report)
        deployment, args = prepare_deployment(config,root,record)
        services = deployment.services
        write_once(root/'role-directories.json', {name:dict(path=str(services.layout[name]),
            identity=list(services.staging_identity[name])) for name in ('common','center','redis')})
        report['phase']='initializing-services'
        _save(root/'report.json',report)
        services.start()
        services_started = True
        report['phase']='waiting-for-templates'
        _save(root/'report.json',report)
        services.wait_ready()
        report.update(services=ci_e2e_artifacts.public_identity(services.manifest),
                      service_boundary=ci_e2e_artifacts.require_boundary(dict(public_api='center',
                          center_unprivileged=True,redis_authority='validated',host_transport='mutual-tls',host_public_management=False)))
        execution, processes = services.execution(config['run_id'], root/'policy.json')
        write_once(root/'execution-manifest.json',execution)
        report.update(execution_manifest=execution, service_processes=processes)
        report['phase']='building-rust-template'
        _save(root/'report.json',report)
        template = build_template(deployment,checkout=ROOT,source=Path(config['rust_envd_acceptance']['oci_archive']),
            profile=Path(envd_acceptance.prepared_paths(config)['profile']),kernel=args.kernel)
        report['rust_template'] = template
        built = json_file(deployment.root/'inputs/rust-build-status.json')
        expected_disk = dict(bytes=built['disk_bytes'], sha256=built['rootfs_sha256'])
        cache = services.host_configs[0]['template_cache']
        original_disk = cached_template(cache, template, expected_disk)
        for name in envd_acceptance.SUITE:
            report['phase']='probe-'+name
            _save(root/'report.json',report)
            report['checks'].append(run_probe(deployment,config,root,name,template))
            _save(root/'report.json',report)
        # The original fault probe emits only its finite JSON report, not logs.
        raw = json_file(root/'faults.log', limit=16*1024**2)
        report['faults'] = raw['faults']
        environment = next(row for row in raw['faults'] if row['name']=='lifecycle')['details']['environment']
        nonce = environment['nonce']
        assert len(nonce)==32 and all(c in '0123456789abcdef' for c in nonce)
        host_path = Path(MARKER + nonce)
        assert not host_path.exists() and not host_path.is_symlink(), 'guest system write reached the host'
        assert cached_template(cache, template, expected_disk) == original_disk, 'shared Rust rootfs changed'
        report['immutable_template'] = dict(**expected_disk, unchanged=True)
        report['environment'] = {key:environment[key] for key in
            ('installed','system_file','fresh_control_pristine','daemon_unchanged')}
        report['environment']['host_system_path_absent'] = True
        report['status'] = 'pass'
    except BaseException as error:
        report['failure_type']=type(error).__name__
        primary = error
    finally:
        finish_worker(deployment, report, root, services_started=services_started, primary=primary)
    if primary is not None:
        raise primary
    return report


def finish_run(config, expected, root, record, code):
    """Finish the exited worker's original cgroup, separately from its cleanup."""
    report=json_file(private_path(root/'report.json'),limit=4*1024**2)
    if (report.get('candidate') != config['candidate'] or report.get('run_id') != config['run_id']
            or report.get('guest') != expected or report.get('resource_cleanup_confirmed') is not True
            or report.get('staging_cleanup_confirmed') is not True):
        raise ValueError('exact candidate, VM or role-directory cleanup remains unknown')
    # A failed/cancelled probe can still have genuinely completed its finally.
    # Remove only the original checked EMPTY cgroup, persist the receipt, then
    # propagate the original failure. Never turn partial checks into a PASS.
    cleanup=service_cgroup.remove(record)
    write_once(root/'service-cleanup.json',cleanup)
    report['service_cleanup_confirmed']=cleanup.get('removed') is True
    _save(root/'report.json',report)
    envd_acceptance.require_cleanup(report,candidate=config['candidate'],run_id=config['run_id'],expected_guest=expected)
    if code!=0:
        raise RuntimeError('Rust envd stage failed; checked cleanup does not make it pass')
    envd_acceptance.require_result(report,candidate=config['candidate'],run_id=config['run_id'],expected_guest=expected)
    return report


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config',type=Path,required=True)
    parser.add_argument('--worker',action='store_true',help=argparse.SUPPRESS)
    args=parser.parse_args(argv)
    config,expected=checked_config(args.config)
    root=Path(config['run_root'])
    if args.worker:
        def interrupted(_number,_frame): raise KeyboardInterrupt('owned envd stage interrupted')
        signal.signal(signal.SIGTERM,interrupted)
        return worker(config,expected)
    root.mkdir(mode=0o700)
    profile=json_file(private_path(config['budget_profile']))['host']
    record=root/'service-cgroup.json'
    service_cgroup.prepare(Path(config['service_cgroup_parent']),'svc-envd-'+secrets.token_hex(8),record,
        cpu_quota_us=profile['control_cpu_allowance']*100000,
        memory_bytes=profile['control_memory_allowance_bytes'])
    command=[*service_cgroup.prefix(record),sys.executable,str(Path(__file__)), '--config',str(args.config),'--worker']
    interrupted=None
    with (root/'worker.log').open('xb') as output:
        process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=output,stderr=output,
                                 env={'PATH':'/usr/local/bin:/usr/bin:/bin','LANG':'C.UTF-8'},start_new_session=True)
        try:
            code=process.wait(timeout=GROUP_SECONDS)
        except (subprocess.TimeoutExpired,KeyboardInterrupt) as error:
            interrupted=error
            process.terminate()
            code=process.wait(timeout=30)
    try:
        report=finish_run(config,expected,root,record,code)
    except BaseException as error:
        if interrupted is not None:
            raise interrupted from error
        raise
    if interrupted is not None:
        raise interrupted
    return report


if __name__ == '__main__':
    main()
