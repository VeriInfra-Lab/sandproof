"""Observe one original benchmark create through checked center/host audits.

No raw Redis/SQLite reads, guessed IDs, public-list inference or create retries.
These diagnostic projections never authorize or release resources. The caller
still must check exact physical resources and retain evidence on uncertainty.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time


BENCHMARKS = frozenset(('terminal-bench-2.0', 'terminal-bench@2.1'))


def _template_prefix(benchmark, prepared_ci):
    # This authority is supplied by the trusted harness, never inferred from
    # a request/checkpoint. The ordinary publication path stays tpl- only.
    if (type(prepared_ci) is not bool or benchmark not in BENCHMARKS
            or (prepared_ci and benchmark != 'terminal-bench-2.0')):
        raise ValueError('invalid explicit runtime recovery mode')
    return 'ci-' if prepared_ci else 'tpl-'


def load_runtime_intent(directory, *, task, runtime_attempt, build_id, template,
                        benchmark='terminal-bench-2.0', prepared_ci: bool = False):
    """Read only this live trial's checkpoint, never discover a previous attempt.

    The expected correlation and template come from the supervisor's live
    environment, not from the file. The controller match still checks its own
    namespace, owner and immutable binding before any DELETE can be sent.
    """
    prefix = _template_prefix(benchmark, prepared_ci)
    if (not isinstance(runtime_attempt, str)
            or not re.fullmatch(r'[0-9a-f]{32}', runtime_attempt)
            or not isinstance(build_id, str) or not re.fullmatch(r'[0-9a-f]{32}', build_id)
            or not isinstance(template, dict) or template.get('templateId') != prefix + build_id
            or not isinstance(task, str) or not task):
        raise ValueError('missing live runtime correlation')
    directory = Path(directory).absolute()
    if directory.resolve(strict=True) != directory:
        raise ValueError('runtime intent directory is not private')
    def unique_keys(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate runtime intent field')
            result[key] = value
        return result
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        info = os.fstat(descriptor)
        if info.st_uid != os.geteuid() or info.st_mode & 0o077:
            raise ValueError('runtime intent directory is not private')
        fd = os.open('runtime-intent.json', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                     dir_fd=descriptor)
        with os.fdopen(fd, 'rb') as source:
            info = os.fstat(source.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                    or info.st_nlink != 1 or info.st_mode & 0o022 or info.st_size > 256 * 1024):
                raise ValueError('unsafe runtime intent file')
            raw = source.read(256 * 1024 + 1)
            if len(raw) > 256 * 1024:
                raise ValueError('runtime intent too large')
    finally:
        os.close(descriptor)
    intent = json.loads(raw, object_pairs_hook=unique_keys)
    expected = {'metadata': {'benchmark': benchmark, 'task': task,
        'build': build_id, 'revision': template.get('revision'), 'runtime_attempt': runtime_attempt},
        'template': template}
    if intent != expected:
        raise ValueError('runtime intent differs from live trial')
    return intent


class RuntimeIdentityUnknown(ValueError):
    """No retained original match; absence is not a terminal result."""


def find_runtime_create(audit, metadata, *, deployment):
    """Correlate a private test intent without exporting user metadata.

    Never a permission, idempotency API or cleanup decision. Ambiguity and a
    changed deployment fail; an absent retained record remains UNKNOWN.
    """
    if (type(metadata) is not dict or any(type(k) is not str or type(v) is not str
            for k, v in metadata.items()) or audit.get('deployment') != deployment):
        raise ValueError('exact original test metadata and deployment required')
    checksum = hashlib.sha256(json.dumps(metadata, sort_keys=True, separators=(',', ':'),
        ensure_ascii=False).encode()).hexdigest()
    records = [record for record in audit['sandboxes']
               if record['private'] is False and record['create']['metadata_sha256'] == checksum]
    if not records:
        raise RuntimeIdentityUnknown('original runtime create identity unknown')
    if len(records) != 1:
        raise RuntimeError('ambiguous original runtime create identity')
    identity = records[0]['create']['identity']
    if identity['node']['deployment_id'] != deployment:
        raise ValueError('original runtime create deployment identity mismatch')
    return identity


def terminal_result(center, host, create_identity):
    """Read both ORIGINAL exact durable confirmations or closed frontiers.

    This never releases anything. Missing records, Stopped, unconfirmed results
    and a single advanced frontier remain unknown. The saved create identity
    must have been obtained from its original checked authority before retirement.
    """
    node, window = create_identity['node'], create_identity['request_window']
    if (type(window) is not int or not 0 < window < 2**64 or host.get('node') != node
            or center.get('deployment') != node['deployment_id']):
        raise ValueError('original saved create and both exact node identities required')
    windows = [value for value in center['windows'] if value['node'] == node]
    if len(windows) != 1:
        raise ValueError('original center node frontier unknown or ambiguous')
    floors = (windows[0]['retired_through'], host['retired_through'])
    if any(type(floor) is not int or not 0 <= floor < 2**64 for floor in floors):
        raise ValueError('original checked window frontier required')
    if all(floor >= window for floor in floors):
        return dict(status='retired_terminal_window', identity=create_identity,
                    center_retired_through=floors[0], host_retired_through=floors[1])
    records = [value for value in center['sandboxes'] if value['create']['identity'] == create_identity]
    if len(records) != 1:
        return None
    record = records[0]
    if not record['released'] or not record['operations']:
        return None
    response = record['operations'][-1]['response']
    if response is None or response['fact']['kind'] not in ('cleaned', 'not_accepted'):
        return None
    identity = response['identity']
    immutable = ('protocol_version', 'node', 'tenant_id', 'sandbox_id', 'request_window')
    if any(identity[key] != create_identity[key] for key in immutable):
        raise ValueError('original terminal response changed create binding')
    rows = [value for value in host['operations'] if value['current'] is True
            and value['identity'] == identity]
    if (len(rows) != 1 or record['terminal_confirmation'] != response
            or rows[0]['response'] != response or rows[0]['confirmed'] != response):
        return None
    return dict(status=response['fact']['kind'], identity=create_identity, response=response)


def recover_runtime_create(intent, *, read_center, read_host, deployment, owner, delete,
                           deadline, sandbox_id=None, create_identity=None,
                           delete_dispatched=False, benchmark='terminal-bench-2.0',
                           prepared_ci=False, before_delete=None,
                           monotonic=time.monotonic, sleep=time.sleep):
    """Observe original audits and dispatch at most ONE authorized known-ID delete.

    Callbacks are explicit operator-owned planned audits, not raw store reads.
    A previously dispatched SDK kill is never silently retried. HTTP success or
    404 does not prove cleanup; both original confirmations/frontiers and the
    caller's separate physical audit are still required. Exceptions retain the
    original trial and identity. This tool contains no lifecycle transition.
    """
    prefix = _template_prefix(benchmark, prepared_ci)
    metadata, template = intent['metadata'], intent['template']
    if (type(owner) is not int or not 0 < owner < 2**64
            or type(delete_dispatched) is not bool
            or not re.fullmatch(r'[0-9a-f]{32}', metadata.get('runtime_attempt', ''))
            or metadata.get('benchmark') != benchmark
            or not re.fullmatch(r'[0-9a-f]{32}', metadata.get('build', ''))
            or template['templateId'] != prefix + metadata['build']
            or template['revision'] != metadata.get('revision')):
        raise ValueError('invalid original runtime creation intent')
    expected = dict(instances=1, cpu=template['cpuCount'],
        memory_bytes=template['hostMemoryBytes'], disk_bytes=template['diskBytes'])
    return recover_original_create(metadata, expected_resources=expected,
        read_center=read_center, read_host=read_host, deployment=deployment, owner=owner,
        delete=delete, deadline=deadline, sandbox_id=sandbox_id, create_identity=create_identity,
        delete_dispatched=delete_dispatched, before_delete=before_delete,
        monotonic=monotonic, sleep=sleep)


def recover_original_create(metadata, *, expected_resources, read_center, read_host,
                            deployment, owner, delete, deadline, sandbox_id=None,
                            create_identity=None, delete_dispatched=False, before_delete=None,
                            monotonic=time.monotonic, sleep=time.sleep):
    """Single diagnostic observer for explicitly correlated private system probes.

    Benchmark checkpoints additionally use the strict wrapper above. Neither
    entry bypasses product authentication, creates/retries work or changes an
    authority. Exact physical evidence is the enclosing probe's responsibility.
    """
    if (type(owner) is not int or not 0 < owner < 2**64
            or type(delete_dispatched) is not bool
            or type(expected_resources) is not dict
            or set(expected_resources) != {'instances','cpu','memory_bytes','disk_bytes'}
            or any(type(value) is not int or not 0 < value < 2**64
                   for value in expected_resources.values()) or expected_resources['instances'] != 1):
        raise ValueError('exact original owner and approved resources required')
    outcome = dict(status='unknown', delete_dispatched=delete_dispatched,
                   delete_acknowledged=False, pending_create_observed=False)
    identity = create_identity
    while monotonic() < deadline:
        center = read_center()
        if identity is None:
            try:
                identity = find_runtime_create(center, metadata, deployment=deployment)
            except RuntimeIdentityUnknown:
                sleep(min(.5, max(0, deadline - monotonic())))
                continue
        if (identity['node']['deployment_id'] != deployment
                or identity['tenant_id'] != str(owner)
                or not re.fullmatch(r'sandbox-[1-9][0-9]*', identity['sandbox_id'])
                or (sandbox_id is not None and identity['sandbox_id'] != sandbox_id)):
            raise RuntimeError('original runtime creation identity mismatch')
        outcome.update(sandbox_id=identity['sandbox_id'], create_identity=identity)
        records = [record for record in center['sandboxes']
                   if record['create']['identity'] == identity]
        if len(records) > 1:
            raise RuntimeError('ambiguous original runtime create record')
        if records:
            record = records[0]
            if find_runtime_create(center, metadata, deployment=deployment) != identity:
                raise RuntimeError('original runtime create binding changed')
            if record['private'] or record['create']['resources'] != expected_resources:
                raise RuntimeError('original runtime creation resource binding changed')
            if record['started_at_ms'] is None and not record['released']:
                outcome['pending_create_observed'] = True
        host = read_host(identity['node'])
        terminal = terminal_result(center, host, identity)
        if terminal is not None:
            return outcome | dict(status='destroyed', terminal=terminal)
        # A retained nonterminal global record is the only source of this known
        # identity. Destruction may safely supersede a pending high-level Create;
        # the original host reducer, not this observer, resolves native work.
        if records and not outcome['delete_dispatched']:
            if before_delete is not None:
                if before_delete(identity, center, host) is False:
                    # A probe may need to observe its original live physical
                    # witness before destroying it. This is not a new command;
                    # the same accepted create continues under the real driver.
                    sleep(min(.5, max(0, deadline - monotonic())))
                    continue
            outcome['delete_dispatched'] = True
            try:
                status = delete(identity['sandbox_id'])
                outcome.update(delete_http_status=status, delete_acknowledged=status in (200, 204))
                if status not in (200, 204, 404):
                    raise RuntimeError('original runtime deletion rejected')
            except OSError as error:
                outcome['delete_error_type'] = type(error).__name__
        sleep(min(.5, max(0, deadline - monotonic())))
    raise TimeoutError('original identity or confirmed terminal remains unknown; retain trial')
