"""Run hot branches and an ordinary Full/Diff lifecycle in one approved deployment."""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)['require_assertions']
_require_assertions()

import argparse
import fcntl
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import stat
import time

from _runtime_services import RuntimeServices
from hot_fanout_host import SDK_COMMIT
from hot_fanout_product import (
    COMMAND_MARKER, MARKER, batch_request, check_batch_parent_ended, check_batch_rejection,
    check_batch_replay, check_hot_unload_rejection, check_reverse_branch_writes,
    check_sdk_not_found, check_stale_data_route, concurrent_sdk, start_batch,
)
from lifecycle_unload_quota import validate_run_receipt
from lifecycle_unload_runtime import (
    UnloadRuntime, add_arguments, checked_file, original_json, retained_deployment_lease,
)
from lifecycle_unload_scenario import PublicUnloadScenario, check_center_configuration, failure_details
from sdk_c02_acceptance import processes_in_uid_range
from sdk_server_smoke import options as sdk_options
from server_process_crash import free_loopback_port, process_start_ticks
from tools.capture_acceptance import validate_settings
from tools.check_kvm_host import trusted_path, unique_keys
from tools.system_acceptance import private_path, write_once

GIB = 1024**3


class CombinedRuntime(UnloadRuntime):
    def __init__(self, args, fixture, root, captures, quota, lease, lock, hot_receipt):
        self.hot_receipt = hot_receipt
        super().__init__(args, fixture, root, captures, quota, lease, lock)

    def _prepare(self):
        native = self.inputs['native']
        self.services = RuntimeServices.prepare(
            self.args, self.root, [native], [self.inputs['catalog']], self.owner_key,
            '127.0.0.1:' + str(free_loopback_port()), self.args.service_cgroup_record, 2,
            registry_config=self.inputs['registry'],
            extra_credentials=[dict(tenant=2, secret=self.foreign_key)],
            activation_ceiling_bytes=self.inputs['activation_ceiling_bytes'],
            hot_fanout=self.hot_receipt,
        )


def approved_json(path, expected, limit):
    path = private_path(path)
    if bytes(checked_file(path, limit)['sha256']).hex() != expected:
        raise ValueError('approved exact input fingerprint differs')
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('approved input changed after checking')
    return json.loads(raw, object_pairs_hook=unique_keys)


def prepare_combined(args):
    if os.geteuid() != 0 or not os.access('/dev/kvm', os.R_OK | os.W_OK):
        raise ValueError('approved Linux KVM execution required')
    os.umask(0o077)
    sdk = importlib.metadata.distribution('e2b')
    if sdk.version != '2.51.0' or json.loads(sdk.read_text('direct_url.json'))['vcs_info']['commit_id'] != SDK_COMMIT:
        raise ValueError('the pinned original E2B SDK is required')
    root = private_path(args.run_root, directory=True)
    fixture = approved_json(args.fixture, args.fixture_sha256, GIB // 1024)
    hot_receipt = approved_json(args.hot_receipt, args.hot_receipt_sha256, 65536)
    if 'fault_case' in hot_receipt or not 2 <= args.n <= hot_receipt['native_hot_fanout']['template']['max_branches']:
        raise ValueError('non-faulting approved hot fixture covering requested N required')
    if set(fixture) != {'native', 'catalog', 'registry', 'activation_ceiling_bytes',
                         'startup_timeout_ms', 'timeout_ms', 'max_lifetime_ms'}:
        raise ValueError('exact bounded lifecycle deployment fixture required')
    templates = fixture['catalog']['templates']
    if len(templates) != 2 or {row['owner'] for row in templates} != {1, 2}:
        raise ValueError('distinct hot and ordinary template identities required')
    [hot] = [row for row in templates if row['owner'] == 1]
    [ordinary] = [row for row in templates if row['owner'] == 2]
    if hot != hot_receipt['binding'] or hot['template_id'] == ordinary['template_id']:
        raise ValueError('single approved hot template and a distinct ordinary template required')
    resources = ordinary['resources']
    native = fixture['native']
    if (resources['memory_mib'] != 2048 or resources['disk_bytes'] != 10 * GIB
            or native.get('enable_memory_diff') is not True):
        raise ValueError('ordinary 2 GiB RAM/10 GiB disk with active Diff is required')
    captures = validate_settings(original_json(args.capture_settings, 65536), 1)
    quota = approved_json(args.quota_receipt, args.quota_receipt_sha256, 65536)
    deployment = approved_json(args.deployment_receipt, args.deployment_receipt_sha256, 65536)
    validate_run_receipt(quota, deployment, captures, args.deployment_receipt_sha256)
    mount = Path(quota['mount'])
    control = private_path(quota['deployment_control'], directory=True)
    staging = trusted_path(args.service_staging_parent)
    coordination = Path(native['bounded_host']['physical_storage']['coordination_file'])
    project_device = quota['filesystem_device']
    if (not mount.is_mount() or control != args.quota_receipt.parent or root == mount
            or not root.is_relative_to(mount) or not control.is_relative_to(mount)
            or staging == mount or not staging.is_relative_to(mount)
            or any(path.stat().st_dev != project_device for path in (root, control, staging))
            or Path(native['database']) != root / 'db/controller.db'
            or Path(native['node_state_dir']) != root / 'state'
            or Path(captures[0]['layout_parent']) != root / 'p'
            or Path(native['chroot_base']) != root / 'p/v'
            or coordination.parent.stat().st_dev != project_device):
        raise ValueError('runtime must belong to the approved XFS installation and exact run paths')
    for name, expected in quota['raw_output_sha256'].items():
        if bytes(checked_file(control / (name + '.txt'), 65536)['sha256']).hex() != expected:
            raise ValueError('original per-trial quota readback changed')
    slots = native['bounded_host']['maintenance']['max_host_slots']
    if (processes_in_uid_range(native['uid_base'], native['uid_base'] + slots - 1)
            or any(processes_in_uid_range(uid, uid) for uid in (args.center_uid, args.redis_uid))):
        raise ValueError('original exclusive test UID domains must be unused before launch')
    lease = retained_deployment_lease(args, quota)
    lock = os.open(args.native_lock, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        meta = os.fstat(lock)
        if (not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1 or meta.st_mode & 0o077
                or dict(device=meta.st_dev, inode=meta.st_ino, owner_uid=meta.st_uid) != quota['native_lock_identity']):
            raise ValueError('original approved native lock changed')
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if not lease.exclusive():
            raise ValueError('original exclusive HostLease no longer has an idle host')
        write_once(control / 'scenario-starting.json', dict(identity=lease.identity,
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            pid=os.getpid(), start_time_ticks=process_start_ticks(os.getpid())))
        return CombinedRuntime(args, fixture, root, captures, quota, lease, lock, hot_receipt)
    except BaseException:
        os.close(lock)
        raise


def assert_capture_mode(physical, source, mode):
    bundle, completion_path = map(Path, physical['material_paths'])
    key = bundle.name.removeprefix('capture-')
    completion = original_json(completion_path, 16384)
    returned = original_json(bundle.parent / ('api-returned-' + key), 16384)
    request = json.dumps(dict(snapshot_type=mode, snapshot_path=f'/full-{key}/state.bin',
        mem_file_path=f'/full-{key}/memory.bin'), sort_keys=True, separators=(',', ':')).encode()
    original = {field: source[field] for field in ('pid', 'start_time_ticks', 'boot_id')}
    assert returned['version'] == 1
    assert returned['source'] == original and returned['resource_id'] == source['resource_id']
    assert returned['worker'] == completion['worker']
    assert returned['context']['job_sha256'] == completion['job_digest']
    assert returned['directory_key'] == list(bytes.fromhex(key))
    assert returned['outcome']['state'] == 'returned' and 200 <= returned['outcome']['http_status'] < 300
    assert returned['request_sha256'] == list(hashlib.sha256(request).digest())
    return dict(kind=mode, directory_key=key, request_digest_verified=True)


def exercise_hot(runtime, client, args, report):
    receipt = runtime.hot_receipt
    pin = receipt['native_hot_fanout']['template']
    checkpoint = pin['checkpoint_id']
    report['create'] = client.create(runtime.create_parameters(owner=1))
    running = client.wait_running()
    original = runtime.record_running(client.session, client.deadline)
    assert original['local']['owner'] == 1
    report['idle_unload_status'] = check_hot_unload_rejection(client, running)
    request = batch_request(client, running, checkpoint, args.n)
    path, operation, branches = start_batch(client, running, checkpoint, args.n, report)
    report['same_key_changed_count_status'] = check_batch_replay(client, path, operation, request, branches)
    report['capacity_status'] = check_batch_rejection(client, running, checkpoint, pin['max_branches'] - args.n + 1, 429)
    report['active_unload_status'] = check_hot_unload_rejection(client, running)
    services = runtime.services
    options = sdk_options(services.key, services.sdk_url, services.sdk_url)
    sandboxes = concurrent_sdk(branches, options, receipt['expected_inherited_mark'], report)
    check_reverse_branch_writes(client, sandboxes)
    parent = client.sdk()
    parent.commands.run(f'test ! -e {MARKER} && test ! -e {COMMAND_MARKER}', user='root', timeout=10)
    report['same_original_vm'] = runtime.confirm_running(client.session, original, client.deadline)
    report['destroy'] = client.destroy(running)
    client.wait_cleaned()
    check_batch_parent_ended(client, path, operation)
    for branch, sandbox in zip(branches, sandboxes, strict=True):
        check_sdk_not_found(branch, options)
        check_stale_data_route(client, sandbox)
    report['terminal'] = runtime.destroy_evidence(client.session, client.deadline)


def exercise_ordinary(runtime, client, report):
    report['create'] = client.create(runtime.create_parameters(owner=2))
    running = client.wait_running()
    client.assert_owner_isolation()
    report['workload'] = client.run_public_workload(running)
    original = runtime.record_running(client.session, client.deadline)
    report['cycles'] = []
    for mode in ('Full', 'Diff'):
        row = dict(unload=client.unload(running, concurrent_duplicate=True))
        report['cycles'].append(row)
        client.assert_data_closed()
        saved = client.wait_unloaded()
        physical = runtime.unload_evidence(client.session, client.deadline, row['unload']['key'])
        client.assert_unloaded_facts(saved, physical['facts'])
        sizes = {item['name']: item['bytes'] for item in physical['materials']}
        assert (sizes['memory.bin'], sizes['disk-0.img']) == (2 * GIB, 10 * GIB)
        client.assert_original_replay(saved)
        row['physical'] = physical
        row['capture_request'] = assert_capture_mode(physical, original['source'], mode)
        row['resume'] = client.resume(saved)
        running = client.wait_running()
        original = runtime.record_running(client.session, client.deadline, resume_acceptance=row['resume'])
        assert original['running_backing']['manifest']['sha256'] == physical['facts']['manifest_digest']
        row['restored_vmm'] = original
        row['same_guest'] = client.assert_resumed_workload(running, physical_running=original)
    assert report['cycles'][0]['capture_request']['directory_key'] != report['cycles'][1]['capture_request']['directory_key']
    report['destroy'] = client.destroy(running)
    client.wait_cleaned()
    report['terminal'] = runtime.destroy_evidence(client.session, client.deadline)


def run(args):
    runtime = prepare_combined(args)
    deadline = time.monotonic() + args.scenario_timeout
    report = dict(schema=1, kind='hot-and-ordinary-shared-deployment', complete=False,
        host_lease_identity=runtime.lease.identity, sdk_commit=SDK_COMMIT, requested_branches=args.n,
        hot_fixture_sha256=args.hot_receipt_sha256, phase='configuration')
    clients = []
    try:
        report['measurement_identity'] = runtime.measurement_identity()
        registration = check_center_configuration(runtime.services, runtime.owner_key, runtime.foreign_key)
        deployment = registration['deployment']
        report['registration'] = registration
        runtime.start(deadline)
        current = PublicUnloadScenario(runtime.services.sdk_url, runtime.owner_key, runtime.foreign_key,
            deployment, deadline)
        clients.append(current)
        report['phase'] = 'hot-template'
        exercise_hot(runtime, current, args, report.setdefault('hot', {}))
        current = PublicUnloadScenario(runtime.services.sdk_url, runtime.foreign_key, runtime.owner_key,
            deployment, deadline, owner='2')
        clients.append(current)
        report['phase'] = 'ordinary-template'
        exercise_ordinary(runtime, current, report.setdefault('ordinary', {}))
        report['phase'] = 'settlement'
        report['settlement'] = runtime.settlement_evidence(deadline)
        report['complete'] = True
    except Exception as error:
        report['failure'] = dict(error=type(error).__name__, **failure_details(
            error, clients[-1] if clients else None, report['phase']))
    finally:
        report['known_sessions'] = [dict(owner=client.owner, session=client.session,
            window=client.window, sequence=client.sequence, acceptance=client.current_acceptance,
            timings=client.timing_report()) for client in clients]
        try:
            runtime.finish(report)
        except Exception as error:
            report['complete'] = False
            report['finish_failure'] = type(error).__name__
        finally:
            try:
                write_once(runtime.root / 'hot-fanout-combined-report.json', report)
            finally:
                os.close(runtime.lock)
    return 0 if report['complete'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument('--fixture-sha256', required=True)
    parser.add_argument('--hot-receipt', type=Path, required=True)
    parser.add_argument('--hot-receipt-sha256', required=True)
    parser.add_argument('--n', type=int, required=True)
    parser.add_argument('--scenario-timeout', type=int, default=600)
    args = parser.parse_args()
    if not 0 < args.scenario_timeout <= 3600:
        parser.error('original finite scenario timeout up to 3600s is required')
    return run(args)


if __name__ == '__main__':
    raise SystemExit(main())
