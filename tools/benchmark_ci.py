"""Trusted E2E policy, result reduction and exact ephemeral storage cleanup.

No GitHub input is a shell command or an authorization. Only the operator's
root-owned policy permits a candidate. A batch's success includes final service
and proxy cleanup, not merely the SDK kill response or a child exit code.
"""
import copy
import hashlib
import os
from pathlib import Path
import re
import stat

from tools.benchmark_tasks import PIN21, canonical, digest, InvalidInput
from tools.bounded_acceptance import bounded_acceptance_layout
from tools.system_acceptance import json_file

CHECK_NAME = 'E2E / terminal-bench-2.1-subset'


def private_path(path, *, directory=False):
    path = Path(path)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError('canonical private path required')
    details = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if not kind(details.st_mode) or details.st_uid != os.geteuid() or details.st_mode & 0o077:
        raise ValueError('owner-only regular file or directory required')
    return path


def subset(lock, manifest):
    names = [row['name'] for row in manifest['tasks']]
    changed = manifest['changed_tasks']
    if (lock['dataset'] != 'terminal-bench@2.1' or lock['source']['commit'] != PIN21.commit
            or manifest['schema_version'] != 1 or manifest['dataset'] != lock['dataset']
            or manifest['source_commit'] != PIN21.commit or manifest['batch_size'] != 4
            or manifest.get('source_kind') != 'dockerfile'
            or len(names) != 8 or len(set(names)) != 8
            or len(changed) != 28 or len(set(changed)) != 28
            or not set(names) <= set(changed) <= {row['name'] for row in lock['tasks']}):
        raise InvalidInput('subset differs from the reviewed eight-task scope')
    return [names[:4], names[4:]]


def run_identity(candidate, base, configuration, manifest, lock):
    for value in (candidate, base):
        if not isinstance(value, str) or re.fullmatch('[0-9a-f]{40}', value) is None:
            raise ValueError('exact commit identity required')
    return digest(canonical(dict(candidate=candidate, base=base,
        config=configuration, subset=manifest, lock=digest(canonical(lock)))))


def authorize(candidate, *, approved_commits, observed_test=None):
    if re.fullmatch('[0-9a-f]{40}', candidate or '') is None:
        raise ValueError('invalid candidate identity')
    if candidate not in approved_commits and candidate != observed_test:
        raise ValueError('candidate is not an approved commit or trusted test head')


def instantiate_policy(template, root, namespaces):
    """Only layout changes; the reviewed resource/capacity values are unchanged."""
    policy = copy.deepcopy(template)
    profiles = [policy[role]['bounded_host']['physical_storage'] for role in ('runtime', 'builder')]
    if profiles[0] != profiles[1]:
        raise ValueError('template profiles must be identical')
    profile = profiles[0]
    root = Path(root)
    profile['coordination_file'] = str(root / 'coordination')
    for index, role in enumerate(('runtime', 'builder')):
        pool = next(p for p in profile['pools'] if p['role'] == role)
        pool.update(namespace=namespaces[index], database=str(root / str(index) / 'db/controller.db'),
                    node_state_dir=str(root / str(index) / 'state'), chroot_base=str(root / str(index) / 'jails'))
    profile['public_build'].update(root=str(root / 'builds'), registry_root=str(root / 'registry'))
    for role in ('runtime', 'builder'):
        policy[role]['bounded_host']['physical_storage'] = copy.deepcopy(profile)
    bounded_acceptance_layout(policy, run_root=root, namespace=namespaces[0], build_namespace=namespaces[1])
    return policy


def batch_result(root, *, candidate, names, exit_code, proxy_required=True, expected_hashes=None):
    report = json_file(root / 'report.json', limit=4 * 1024 * 1024)
    service = json_file(root / 'service-cleanup.json')
    rows = report.get('tasks', [])
    observed = [r.get('name') for r in rows]
    if (report.get('commit') != candidate or report.get('dataset') != 'terminal-bench@2.1'
            or report.get('task_source') != {'repository': PIN21.repository, 'commit': PIN21.commit}
            or report.get('fixed_task_count') != 89 or len(observed) != len(names)
            or len(set(observed)) != len(names) or set(observed) != set(names)
            or report.get('source_kind') != 'dockerfile'
            or report.get('original_environment') is not True or report.get('paid_model_used') is not False
            or (root / 'service-cleanup-errors.json').exists()
            or service.get('removed') is not True
            or any(r.get('exact_cleanup') != 'passed' for r in rows)
            or report.get('no_process_network_cgroup_jail_residue') is not True):
        raise ValueError('incomplete, mismatched or unknown batch cleanup evidence')
    if proxy_required and json_file(root / 'proxy-cleanup.json').get('phase') != 'removed':
        raise ValueError('proxy cleanup is unknown')
    fingerprint_fields = ('server_sha256', 'kernel_sha256', 'builder_base_sha256')
    if (any(not isinstance(report.get(key), str) or re.fullmatch('[0-9a-f]{64}', report[key]) is None
            for key in fingerprint_fields)
            or (expected_hashes is not None and any(report.get(key) != value
                                                    for key, value in expected_hashes.items()))):
        raise ValueError('batch artifacts differ from the prepared host inputs')
    # Construct an allowlist. Never copy config, paths, raw logs or exceptions.
    statuses = {'passed', 'failed', 'timeout', 'interrupted', 'cleanup_unknown', 'runner_error', 'not_run'}
    safe = []
    for row in rows:
        item = dict(name=row['name'], status=row.get('status') if row.get('status') in statuses else 'unknown',
            oracle=row.get('oracle') if row.get('oracle') in statuses else 'not_run',
            verifier=row.get('verifier') if row.get('verifier') in statuses else 'not_run',
            exact_cleanup=row['exact_cleanup'],
            rewards=row.get('rewards') if row.get('rewards') in ({'reward': 0}, {'reward': 1}) else None)
        build = row.get('build_id')
        if isinstance(build, str) and re.fullmatch('[0-9a-f]{32}', build):
            item['build_id'] = build
        template = row.get('template') or (row.get('build') or {}).get('template') or {}
        item['template_hashes'] = {key: value for key, value in template.items()
            if key in ('rootfsSha256', 'kernelSha256') and isinstance(value, str)
            and re.fullmatch('[0-9a-f]{64}', value)}
        if set(item['template_hashes']) != {'rootfsSha256', 'kernelSha256'}:
            raise ValueError('immutable template fingerprints are missing')
        safe.append(item)
    passed = exit_code == 0 and all(r['status'] == r['oracle'] == r['verifier'] == 'passed'
        and r['rewards'] == {'reward': 1} for r in safe)
    hashes = {key: value for key, value in report.items() if key in
              ('server_sha256', 'kernel_sha256', 'builder_base_sha256')
              and isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value)}
    return dict(tasks=safe, passed=passed, cleanup='passed', exit_code=exit_code, artifact_hashes=hashes,
                report_sha256=hashlib.sha256((root / 'report.json').read_bytes()).hexdigest())


def aggregate(candidate, batches, expected):
    rows = [row for batch in batches for row in batch['tasks']]
    names = [row['name'] for row in rows]
    complete = len(names) == len(expected) and len(set(names)) == len(expected) and set(names) == set(expected)
    passed = complete and all(batch['passed'] is True and batch['cleanup'] == 'passed' for batch in batches)
    return dict(schema_version=1, candidate=candidate, dataset='terminal-bench@2.1',
        scope='eight-task regression subset; not full-suite acceptance',
        required_tasks=list(expected), tasks=rows, complete=complete, passed=passed,
        executed_task_count=len(rows), full_suite_passed=False, paid_model_used=False)


def remove_private_tree(root, expected_identity):
    """Delete only an audited run root, never follow links or mount crossings.

    Caller MUST first establish process/network/cgroup absence. Pre-scan every
    object before unlinking anything. Unknown types or owners retain evidence.
    This is ephemeral CI fixture cleanup, not product template/history GC.
    """
    root = private_path(root, directory=True)
    info = root.lstat()
    if [info.st_dev, info.st_ino] != expected_identity or root.parent == Path('/'):
        raise ValueError('run root identity changed')
    inventory = []
    def scan(fd, parts=()):
        for name in os.listdir(fd):
            item = os.stat(name, dir_fd=fd, follow_symlinks=False)
            if item.st_dev != info.st_dev or item.st_uid != os.geteuid():
                raise ValueError('unknown owner or mount in run root')
            if stat.S_ISDIR(item.st_mode):
                child = os.open(name, os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                try:
                    scan(child, parts + (name,))
                finally:
                    os.close(child)
            elif not stat.S_ISREG(item.st_mode):
                raise ValueError('unknown file type in run root')
            inventory.append((parts + (name,), item.st_dev, item.st_ino, stat.S_ISDIR(item.st_mode)))
    fd = os.open(root, os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        scan(fd)
        for parts, device, inode, directory in inventory:
            parent = os.dup(fd)
            try:
                for name in parts[:-1]:
                    child = os.open(name, os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                    os.close(parent)
                    parent = child
                actual = os.stat(parts[-1], dir_fd=parent, follow_symlinks=False)
                if (actual.st_dev, actual.st_ino) != (device, inode):
                    raise ValueError('file identity changed during cleanup')
                (os.rmdir if directory else os.unlink)(parts[-1], dir_fd=parent)
            finally:
                os.close(parent)
        if (root.stat().st_dev, root.stat().st_ino) != tuple(expected_identity):
            raise ValueError('run root replaced during cleanup')
        root.rmdir()
    finally:
        os.close(fd)
