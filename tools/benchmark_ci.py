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

from tools.benchmark_tasks import PIN21, canonical, digest, InvalidInput, pin_for
from tools.bounded_acceptance import bounded_acceptance_layout
from tools.system_acceptance import json_file, private_path
from benchmarks.ci_verifier import (BOOTSTRAP_SHA256, LAUNCHER, MODE as CI_VERIFIER_MODE,
                                    REQUIREMENTS_SHA256)

CHECK_NAME = 'E2E / terminal-bench-2.1'


def ci_verifier_evidence(row):
    """Require the reviewed local bootstrap, export no sidecar paths or inventories."""
    expected = dict(mode=CI_VERIFIER_MODE, source_launcher_sha256=BOOTSTRAP_SHA256,
        launcher_sha256=hashlib.sha256(LAUNCHER.encode()).hexdigest(), requirements_sha256=REQUIREMENTS_SHA256)
    variant = row.get('verifier_variant')
    if (row.get('guest_preflight') != 'passed' or row.get('verifier_mode') != CI_VERIFIER_MODE
            or not isinstance(variant, dict) or any(variant.get(key) != value for key, value in expected.items())):
        raise ValueError('prepackaged CI guest preflight or verifier provenance is incomplete or changed')
    return dict(guest_preflight='passed', verifier_mode=CI_VERIFIER_MODE, verifier_variant=expected)


def check_name(dataset, *, source_kind=None):
    pin = pin_for({'dataset': dataset})
    if source_kind == 'prepared-ci':
        if pin.version != '2.0':
            raise ValueError('prepackaged CI is bound to the reviewed 2.0 dataset')
        return 'E2E / terminal-bench-ci'
    return 'E2E / terminal-bench-' + pin.version + ('-compat' if pin.version == '2.0' else '')


def oracle_modes(lock, manifest):
    """Only explicit, hash-matched legacy repairs; never alter product behavior."""
    from benchmarks.oracle_repairs import REPAIRS
    entries = {t['name']: t for t in lock['tasks']}
    overrides = manifest.get('oracle_modes', {})
    if (not isinstance(overrides, dict) or not set(overrides) <= entries.keys()
            or any(v != 'repaired' for v in overrides.values())
            or (lock['dataset'] != 'terminal-bench@2.0' and overrides)):
        raise InvalidInput('unreviewed oracle policy')
    for name in overrides:
        original = next((f for f in entries[name]['files'] if f['path'] == 'solution/solve.sh'), {})
        if name not in REPAIRS or original.get('sha256') != REPAIRS[name]:
            raise InvalidInput('oracle repair differs from the fixed original solution')
    return {n: overrides.get(n, 'original') for n in entries}


def selection(lock, manifest, policy):
    """Resolve an operator-approved profile or exact list; never sample at run time.

    Names, order and cohort size are configuration, not product logic. The
    immutable full lock still supplies every original task's inputs/resources.
    """
    all_names = [row['name'] for row in lock['tasks']]
    pin = pin_for(lock)
    if (lock['source'] != {'repository': pin.repository, 'commit': pin.commit}
            or manifest.get('schema_version') != 2 or manifest.get('dataset') != lock['dataset']
            or manifest.get('source_commit') != pin.commit
            or manifest.get('source_kind') not in ('oci', 'dockerfile')
            or len(set(all_names)) != len(all_names) or not all_names):
        raise InvalidInput('selection catalog differs from the fixed dataset')
    if set(policy) not in ({'profile', 'batch_size'}, {'tasks', 'batch_size'}):
        raise InvalidInput('choose exactly one profile or explicit task list')
    size = policy['batch_size']
    if type(size) is not int or not 1 <= size <= len(all_names):
        raise InvalidInput('batch size must fit the fixed dataset')
    if 'profile' in policy:
        name = policy['profile']
        if not isinstance(name, str) or name not in manifest['profiles']:
            raise InvalidInput('unknown task profile')
        profile = manifest['profiles'][name]
        names = list(all_names) if profile.get('all') is True else [row['name'] for row in profile['tasks']]
    else:
        names = policy['tasks']
    if (not isinstance(names, list) or not names or any(not isinstance(n, str) for n in names)
            or len(names) != len(set(names)) or not set(names) <= set(all_names)):
        raise InvalidInput('empty, duplicate or unknown task selection')
    modes = oracle_modes(lock, manifest)
    batches = []
    for name in names:
        # The existing harness accepts one mode per batch. Split rather than
        # rewrite solutions silently or mix incompatible result identities.
        if not batches or len(batches[-1]) == size or modes[batches[-1][0]] != modes[name]:
            batches.append([])
        batches[-1].append(name)
    return batches


def run_identity(candidate, base, configuration, manifest, lock):
    for value in (candidate, base):
        if not isinstance(value, str) or re.fullmatch('[0-9a-f]{40}', value) is None:
            raise ValueError('exact commit identity required')
    return digest(canonical(dict(candidate=candidate, base=base,
        config=configuration, subset=manifest, lock=digest(canonical(lock)))))


def authorize(candidate, *, approved_commits, observed_promotion=None):
    if re.fullmatch('[0-9a-f]{40}', candidate or '') is None:
        raise ValueError('invalid candidate identity')
    if observed_promotion is not None:
        if (not isinstance(observed_promotion, tuple) or len(observed_promotion) != 3
                or observed_promotion[:2] not in (('integration', 'test'), ('test', 'main'), ('main', 'test'))
                or observed_promotion[2] != candidate):
            raise ValueError('invalid protected promotion authority')
    if candidate not in approved_commits and observed_promotion is None:
        raise ValueError('candidate is not an approved commit or trusted test head')


def cohort_policy(template, job_count):
    """Bind exact batch counters within approved operator ceilings.

    Partial cohorts must match the harness's derived counts. Compute, byte and
    cleanup ceilings are unchanged; no extra resource authority is added.
    """
    policy = copy.deepcopy(template)
    profiles = [policy[role]['bounded_host']['physical_storage'] for role in ('runtime', 'builder')]
    if profiles[0] != profiles[1]:
        raise ValueError('template profiles must be identical')
    profile = profiles[0]
    public = profile['public_build']
    if type(job_count) is not int or not 1 <= job_count <= min(public['max_jobs'], public['max_templates']):
        raise ValueError('batch exceeds the approved retained-job/template ceilings')
    public.update(max_jobs=job_count, max_templates=job_count)
    for role in ('runtime', 'builder'):
        policy[role]['bounded_host']['physical_storage'] = copy.deepcopy(profile)
    return policy


def instantiate_policy(template, root, namespaces, *, job_count):
    """Bind layout and exact counts; retain all approved byte ceilings."""
    policy = cohort_policy(template, job_count)
    profile = policy['runtime']['bounded_host']['physical_storage']
    root = Path(root)
    profile['coordination_file'] = str(root / 'coordination')
    for index, role in enumerate(('runtime', 'builder')):
        pool = next(p for p in profile['pools'] if p['role'] == role)
        original_namespace = pool['namespace']
        pool.update(namespace=namespaces[index], database=str(root / str(index) / 'db/controller.db'),
                    node_state_dir=str(root / str(index) / 'state'), chroot_base=str(root / str(index) / 'jails'))
        for cache in profile['host_template_caches']:
            if cache['namespace'] == original_namespace:
                cache.update(namespace=namespaces[index], root=str(root / str(index) / 'template-cache'))
    profile['public_build'].update(root=str(root / 'builds'), registry_root=str(root / 'registry'))
    for role in ('runtime', 'builder'):
        policy[role]['bounded_host']['physical_storage'] = copy.deepcopy(profile)
    bounded_acceptance_layout(policy, run_root=root, namespace=namespaces[0], build_namespace=namespaces[1])
    return policy


def batch_result(root, *, candidate, names, exit_code, proxy_required=True, expected_hashes=None,
                 full_task_count, dataset='terminal-bench@2.1', source_kind='dockerfile', oracle_mode='original',
                 expected_fixture_sha256=None, service_manifest, dk_evidence):
    from tools.ci_e2e_artifacts import (require_service_boundary, require_execution_manifest,
        require_process_evidence,require_dk)
    pin = pin_for({'dataset': dataset})
    prepared = source_kind == 'prepared-ci'
    if prepared:
        if (pin.version != '2.0' or oracle_mode != 'original'
                or not isinstance(expected_fixture_sha256, str)
                or re.fullmatch('[0-9a-f]{64}', expected_fixture_sha256) is None):
            raise ValueError('reviewed prepackaged CI fixture identity required')
    elif expected_fixture_sha256 is not None:
        raise ValueError('fixture identity is only valid for prepackaged CI')
    if oracle_mode not in ('original', 'repaired') or (pin.version != '2.0' and oracle_mode != 'original'):
        raise ValueError('unreviewed batch oracle mode')
    report = json_file(root / 'report.json', limit=4 * 1024 * 1024)
    boundary = require_service_boundary(report, service_manifest)
    if boundary['services']['candidate'] != candidate:
        raise ValueError('runtime service candidate differs from the requested commit')
    dk=require_dk(dk_evidence,candidate=candidate)
    execution_path=private_path(root/'execution-manifest.json')
    execution=require_execution_manifest(report.get('execution_manifest'),service_manifest,
        expected=json_file(execution_path,limit=32768))
    processes=require_process_evidence(report.get('service_processes'),execution)
    service = json_file(root / 'service-cleanup.json')
    rows = report.get('tasks', [])
    observed = [r.get('name') for r in rows]
    grading_matches = (report.get('original_verifier') is True
        and report.get('original_oracle_verifier') is (oracle_mode == 'original')
        and report.get('original_environment') is True)
    if prepared:
        grading_matches = (report.get('original_environment') is False
            and report.get('original_verifier') is False
            and report.get('original_oracle_verifier') is False
            and report.get('original_test_assertions') is True
            and report.get('original_oracle') is True
            and report.get('fixture_sha256') == expected_fixture_sha256)
        physical = report.get('prepared_ci_physical', {})
        if (not isinstance(physical, dict) or physical.get('pool_roles') != ['runtime']
                or physical.get('builder_dispatched') is not False or physical.get('public_build') is not False
                or type(physical.get('disk_budget_bytes')) is not int
                or type(physical.get('activation_bytes')) is not int
                or not 0 < physical['activation_bytes'] <= physical['disk_budget_bytes'] <= 200 * 1024**3):
            raise ValueError('prepared CI runtime-only physical budget evidence is incomplete')
    if (report.get('commit') != candidate or report.get('dataset') != dataset
            or report.get('task_source') != {'repository': pin.repository, 'commit': pin.commit}
            or report.get('fixed_task_count') != full_task_count or len(observed) != len(names)
            or len(set(observed)) != len(names) or set(observed) != set(names)
            or report.get('source_kind') != source_kind or source_kind not in ('oci', 'dockerfile', 'prepared-ci')
            or report.get('oracle_mode') != oracle_mode or not grading_matches
            or report.get('paid_model_used') is not False
            or (root / 'service-cleanup-errors.json').exists()
            or service.get('removed') is not True
            or any(r.get('exact_cleanup') != 'passed' or r.get('oracle_mode') != oracle_mode for r in rows)
            or report.get('no_process_network_cgroup_jail_residue') is not True):
        raise ValueError('incomplete, mismatched or unknown batch cleanup evidence')
    if proxy_required and not prepared and json_file(root / 'proxy-cleanup.json').get('phase') != 'removed':
        raise ValueError('proxy cleanup is unknown')
    fingerprint_fields = ('kernel_sha256', 'builder_base_sha256')
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
            oracle_mode=oracle_mode,
            oracle=row.get('oracle') if row.get('oracle') in statuses else 'not_run',
            verifier=row.get('verifier') if row.get('verifier') in statuses else 'not_run',
            exact_cleanup=row['exact_cleanup'],
            rewards=row.get('rewards') if row.get('rewards') in ({'reward': 0}, {'reward': 1}) else None)
        if prepared:
            item.update(ci_verifier_evidence(row))
        build = row.get('build_id')
        if isinstance(build, str) and re.fullmatch('[0-9a-f]{32}', build):
            item['build_id'] = build
        template = row.get('template') or (row.get('build') or {}).get('template') or {}
        item['template_hashes'] = {key: value for key, value in template.items()
            if key in ('rootfsSha256', 'kernelSha256') and isinstance(value, str)
            and re.fullmatch('[0-9a-f]{64}', value)}
        # A terminal failed build has no published template to fingerprint.
        # This is execution failure, not missing cleanup evidence. Only accept
        # that absence when the report explicitly says no runtime was created;
        # successful/publication/runtime paths still require both fingerprints.
        unpublished_failure = (not prepared and item['status'] in ('failed', 'timeout')
            and item['oracle'] == item['verifier'] == 'not_run'
            and row.get('create_dispatched') is False and row.get('sandbox_id') is None
            and (row.get('build') or {}).get('state') == 'Failed'
            and not template)
        if set(item['template_hashes']) != {'rootfsSha256', 'kernelSha256'} and not unpublished_failure:
            raise ValueError('immutable template fingerprints are missing')
        safe.append(item)
    passed = exit_code == 0 and all(r['status'] == r['oracle'] == r['verifier'] == 'passed'
        and r['rewards'] == {'reward': 1} for r in safe)
    hashes = {key: value for key, value in report.items() if key in
              fingerprint_fields
              and isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value)}
    result = dict(candidate=candidate, tasks=safe, dataset=dataset, source_kind=source_kind, oracle_mode=oracle_mode,
                passed=passed, cleanup='passed', exit_code=exit_code, artifact_hashes=hashes,
                report_sha256=hashlib.sha256((root / 'report.json').read_bytes()).hexdigest(),
                execution_manifest_sha256=hashlib.sha256(execution_path.read_bytes()).hexdigest(),
                service_roles=sorted(processes),dk=dk,
                **boundary)
    if prepared:
        result.update(fixture_sha256=expected_fixture_sha256, original_environment=False,
                      original_verifier=False, original_oracle_verifier=False,
                      original_test_assertions=True, original_oracle=True,
                      prepared_ci_physical=physical)
    return result


def aggregate(candidate, batches, expected, *, all_tasks=None, dataset='terminal-bench@2.1',
              source_kind=None, expected_fixture_sha256=None):
    from tools.ci_e2e_artifacts import require_public_identity, require_boundary,require_dk
    service_identity = None
    boundary = None
    dk = None
    for batch in batches:
        identity = require_public_identity(batch.get('services'), candidate=candidate)
        observed = require_boundary(batch.get('service_boundary'))
        faults=require_dk(batch.get('dk'),candidate=candidate)
        if dk is not None and faults!=dk:
            raise ValueError('cannot combine different exact-candidate DK cohorts')
        dk=faults
        if (not isinstance(batch.get('execution_manifest_sha256'),str)
                or re.fullmatch('[0-9a-f]{64}',batch['execution_manifest_sha256']) is None):
            raise ValueError('original dual-service execution receipt missing from durable batch')
        if batch.get('candidate') != candidate or (service_identity is not None and identity != service_identity):
            raise ValueError('cannot combine different center/host candidates or service contracts')
        service_identity, boundary = identity, observed
    pin_for({'dataset': dataset})
    if any(b.get('dataset', dataset) != dataset for b in batches):
        raise ValueError('cannot combine different dataset versions')
    prepared = source_kind == 'prepared-ci' or any(b.get('source_kind') == 'prepared-ci' for b in batches)
    if prepared:
        identities = [b.get('fixture_sha256') for b in batches]
        if (dataset != 'terminal-bench@2.0' or source_kind not in (None, 'prepared-ci')
                or any(b.get('source_kind') != 'prepared-ci' or b.get('oracle_mode') != 'original'
                       or b.get('original_environment') is not False or b.get('original_verifier') is not False
                       or b.get('original_oracle_verifier') is not False
                       or b.get('original_test_assertions') is not True or b.get('original_oracle') is not True
                       for b in batches)
                or any(not isinstance(value, str)
                    or re.fullmatch('[0-9a-f]{64}', value) is None for value in identities)
                or len(set(identities)) > 1
                or (expected_fixture_sha256 is not None and (not isinstance(expected_fixture_sha256, str)
                    or re.fullmatch('[0-9a-f]{64}', expected_fixture_sha256) is None
                    or any(value != expected_fixture_sha256 for value in identities)))):
            raise ValueError('cannot combine different or unreviewed CI fixtures')
        fixture_sha256 = expected_fixture_sha256 or next(iter(identities), None)
        if fixture_sha256 is None:
            raise ValueError('prepackaged CI aggregate requires its fixture identity')
    elif expected_fixture_sha256 is not None:
        raise ValueError('fixture identity is only valid for prepackaged CI')
    rows = [row for batch in batches for row in batch['tasks']]
    if prepared:
        for row in rows:
            ci_verifier_evidence(row)  # Recheck durable batch evidence before terminal publication.
    names = [row['name'] for row in rows]
    complete = len(names) == len(expected) and len(set(names)) == len(expected) and set(names) == set(expected)
    passed = bool(expected) and complete and all(
        batch['passed'] is True and batch['cleanup'] == 'passed' for batch in batches)
    full = (all_tasks is not None and len(expected) == len(all_tasks)
            and len(set(all_tasks)) == len(all_tasks) and set(expected) == set(all_tasks))
    result = dict(schema_version=1, candidate=candidate, dataset=dataset,
        baseline='compatibility regression' if dataset == 'terminal-bench@2.0' else 'original tasks',
        original_oracle_verifier=bool(rows and all(r.get('oracle_mode', 'original') == 'original' for r in rows)),
        scope='full fixed dataset' if full else 'configured regression subset; not full-suite acceptance',
        required_tasks=list(expected), tasks=rows, complete=complete, passed=passed,
        executed_task_count=len(rows), full_suite_passed=bool(full and passed), paid_model_used=False)
    if service_identity is not None:
        result.update(services=service_identity, service_boundary=boundary,dk=dk)
    if prepared:
        result.update(source_kind='prepared-ci', fixture_sha256=fixture_sha256,
            baseline='prepackaged CI regression; original solution and test assertions',
            original_environment=False, original_verifier=False, original_oracle_verifier=False,
            original_test_assertions=True, original_oracle=True,
            scope='prepackaged CI variant; not original or full-suite acceptance', full_suite_passed=False)
    return result


def remove_private_tree(root, expected_identity, *, owner_uid=None):
    """Delete only an audited run root, never follow links or mount crossings.

    Caller MUST first establish process/network/cgroup absence. Pre-scan every
    object before unlinking anything. Unknown types or owners retain evidence.
    This is ephemeral CI fixture cleanup, not product template/history GC.
    """
    if owner_uid is None:
        owner_uid=os.geteuid()
        root = private_path(root, directory=True)
    else:
        # Explicit generated service-role cleanup only. The general private
        # path policy remains unchanged; no old deploy/adoption mode is added.
        root=Path(root)
        if (type(owner_uid) is not int or not 0<=owner_uid<2**32
                or not root.is_absolute() or root.resolve(strict=True)!=root):
            raise ValueError('exact canonical generated role and owner required')
        details=root.lstat()
        if (not stat.S_ISDIR(details.st_mode) or details.st_uid!=owner_uid
                or details.st_mode&0o7777!=0o700):
            raise ValueError('exact protected generated role root required')
    info = root.lstat()
    if [info.st_dev, info.st_ino] != expected_identity or root.parent == Path('/'):
        raise ValueError('run root identity changed')
    inventory = []
    def scan(fd, parts=()):
        for name in os.listdir(fd):
            item = os.stat(name, dir_fd=fd, follow_symlinks=False)
            if item.st_dev != info.st_dev or item.st_uid != owner_uid:
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
