"""Observe one original benchmark create through checked center/host audits.

No raw Redis/SQLite reads, guessed IDs, public-list inference or create retries.
These diagnostic projections never authorize or release resources. The caller
still must check exact physical resources and retain evidence on uncertainty.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time


# Current production ContractVersions only. Center audit schema 3 adds the
# required unloaded projection; host schema 2 retains its existing identity,
# confirmation and retention-lane fields. This observer does not infer material
# cleanup or consume the host's source-work/material configuration.
LIFECYCLE_PROTOCOL_VERSION = 4
CURRENT_VERSIONS = dict(internal_protocol=2, center_store=11, host_store=24)
CENTER_OPERATION_FIELDS = frozenset(('binding', 'center', 'retention',
    'accepted_version', 'acceptance_time', 'operation', 'running',
    'create_refusal', 'terminal', 'unloaded'))

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


def _counter(value, *, zero=False):
    return type(value) is int and int(not zero) <= value < 2**64


def _identifier(value):
    return type(value) is str and re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', value) is not None



def _versions_match(versions):
    return (type(versions) is dict and versions == CURRENT_VERSIONS
        and all(type(value) is int for value in versions.values()))


def _node_shape(node):
    return (type(node) is dict and set(node) == {'deployment_id', 'node_id', 'node_generation'}
        and _identifier(node['deployment_id']) and _identifier(node['node_id'])
        and _counter(node['node_generation']))


def _binding_shape(binding):
    """Current lifecycle identity only; probe/data protocol has its own version."""
    if (type(binding) is not dict or set(binding) != {
            'protocol_version', 'node', 'session', 'key', 'expected'}
            or type(binding['protocol_version']) is not int or binding['protocol_version'] != LIFECYCLE_PROTOCOL_VERSION
            or not _node_shape(binding['node']) or not _identifier(binding['session'])):
        return False
    key, expected = binding['key'], binding['expected']
    return (type(key) is dict and set(key) == {'deployment', 'owner', 'window', 'sequence'}
        and key['deployment'] == binding['node']['deployment_id'] and _identifier(key['owner'])
        and _counter(key['window']) and _counter(key['sequence'])
        and type(expected) is dict
        and (expected == {'state': 'absent'} or
             (set(expected) == {'state', 'version'} and expected['state'] == 'version'
              and _counter(expected['version'], zero=True))))


def _control_shape(identity):
    """Saved fields of the ORIGINAL accepted Create audit, never a permission."""
    if (type(identity) is not dict or set(identity) != {
            'binding', 'center', 'retention', 'accepted_version'}
            or not _binding_shape(identity['binding'])):
        return False
    binding, center, retention = identity['binding'], identity['center'], identity['retention']
    expected = binding['expected']
    version = 1 if expected == {'state': 'absent'} else expected['version'] + 1
    return (type(center) is dict and set(center) == {'deployment', 'center'}
        and center['deployment'] == binding['node']['deployment_id'] and _identifier(center['center'])
        and type(retention) is dict and set(retention) == {'slot', 'window'}
        and type(retention['slot']) is int and 0 < retention['slot'] <= 4096
        and _counter(retention['window']) and _counter(identity['accepted_version'])
        and identity['accepted_version'] == version)


def original_create_identity(record):
    """Copy its first checked operation's identity fields, retaining slot/window.

    OperationKey.window and RetentionBinding.window have different meanings.
    No user options or metadata are exported or reconstructed by this reader.
    """
    operations = record.get('operations')
    if type(operations) is not list or not operations:
        raise ValueError('complete original Create operation required')
    first = operations[0]
    if (type(first) is not dict or set(first) != CENTER_OPERATION_FIELDS
            or first['unloaded'] is not None):
        raise ValueError('complete current original Create operation required')
    identity = {key: first[key] for key in ('binding', 'center', 'retention', 'accepted_version')}
    if (not _control_shape(identity) or identity['accepted_version'] != 1
            or identity['binding']['expected'] != {'state': 'absent'}
            or not _binding_shape(record['create']['binding'])
            or record['create']['binding'] != identity['binding']
            or type(first['operation']) is not dict
            or set(first['operation']) != {'kind', 'target_incarnation'}
            or first['operation']['kind'] != 'create'
            or not _counter(first['operation']['target_incarnation'])):
        raise ValueError('original Create audit identity mismatch')
    return copy.deepcopy(identity)


def find_runtime_create(audit, metadata, *, deployment):
    """Correlate a private test intent against the sole current center audit.

    Not a permission, idempotency API or cleanup decision. Ambiguity and changed
    deployment fail; an absent retained record remains UNKNOWN. Old shapes refuse.
    """
    if (type(metadata) is not dict or any(type(k) is not str or type(v) is not str
            for k, v in metadata.items()) or audit.get('deployment') != deployment
            or type(audit.get('schema_version')) is not int or audit['schema_version'] != 3
            or not _versions_match(audit.get('versions'))):
        raise ValueError('current original test metadata and deployment required')
    checksum = hashlib.sha256(json.dumps(metadata, sort_keys=True, separators=(',', ':'),
        ensure_ascii=False).encode()).hexdigest()
    records = [record for record in audit['sandboxes']
               if record['private'] is False and record['create']['metadata_sha256'] == checksum]
    if not records:
        raise RuntimeIdentityUnknown('original runtime create identity unknown')
    if len(records) != 1:
        raise RuntimeError('ambiguous original runtime create identity')
    identity = original_create_identity(records[0])
    if identity['binding']['node']['deployment_id'] != deployment:
        raise ValueError('original runtime create deployment identity mismatch')
    return identity


def _terminal_shape(response):
    if type(response) is not dict or set(response) != {'request', 'status'}:
        return False
    request = response['request']
    if type(request) is not dict or set(request) != {
            'request', 'accepted_version', 'center', 'retention'}:
        return False
    original = request['request']
    if (type(original) is not dict or set(original) != {'binding', 'operation'}
            or original['operation'] != {'kind': 'destroy'}):
        return False
    identity = dict(binding=original['binding'], center=request['center'],
                    retention=request['retention'], accepted_version=request['accepted_version'])
    return (_control_shape(identity) and original['binding']['expected']['state'] == 'version'
        and response['status'] in ('pending', 'cleaned'))


def _same_session(response, identity):
    request = response['request']
    binding, original = request['request']['binding'], identity['binding']
    return (request['accepted_version'] > identity['accepted_version']
        and binding['node'] == original['node'] and binding['session'] == original['session']
        and binding['key']['owner'] == original['key']['owner']
        and request['center'] == identity['center'] and request['retention'] == identity['retention'])


def terminal_result(center, host, create_identity):
    """Read original confirmations or already retired ORIGINAL retention lanes.

    Host terminal=True is only intent. HTTP success, historical Running, missing
    rows, elapsed time and one advanced frontier cannot establish Cleaned. This
    diagnostic never refunds or replaces separate exact physical observations.
    """
    if (not _control_shape(create_identity) or create_identity['accepted_version'] != 1
            or create_identity['binding']['expected'] != {'state': 'absent'}):
        raise ValueError('complete typed saved create binding required')
    node = create_identity['binding']['node']
    slot, window = (create_identity['retention'][key] for key in ('slot', 'window'))
    versions = center.get('versions')
    if (type(center.get('schema_version')) is not int or center['schema_version'] != 3
            or type(host.get('schema_version')) is not int or host['schema_version'] != 2
            or type(versions) is not dict or versions != host.get('versions')
            or set(versions) != {'internal_protocol', 'center_store', 'host_store'}
            or any(type(value) is not int or value <= 0 for value in versions.values())
            or not _versions_match(versions) or not _versions_match(host.get('versions'))
            or not _node_shape(host.get('node')) or host['node'] != node
            or center.get('deployment') != node['deployment_id']):
        raise ValueError('current audits and original saved create binding required')
    lanes = ([value for value in center['retention_lanes']
              if value['node'] == node and value['retention_slot'] == slot],
             [value for value in host['retention_lanes'] if value['slot'] == slot])
    if any(len(values) != 1 for values in lanes):
        raise ValueError('original retention slot frontier unknown or ambiguous')
    pair = tuple(values[0] for values in lanes)
    if (type(pair[0]['retention_slot']) is not int or type(pair[1]['slot']) is not int
            or not _node_shape(pair[0]['node'])):
        raise ValueError('typed original retention slot binding required')
    floors = tuple(value['retired_through'] for value in pair)
    if any(not _counter(floor, zero=True) for floor in floors):
        raise ValueError('original checked slot frontier required')
    for lane, floor in zip(pair, floors):
        stamp = lane.get('last_retired')
        if floor == 0:
            if stamp is not None:
                raise ValueError('empty slot has a retired stamp')
            continue
        if (not _terminal_shape(stamp) or stamp['status'] != 'cleaned'
                or stamp['request']['request']['binding']['node'] != node
                or stamp['request']['center'] != create_identity['center']
                or stamp['request']['retention'] != {'slot': slot, 'window': floor}):
            raise ValueError('original exact retired terminal stamp required')
        if floor == window and not _same_session(stamp, create_identity):
            raise ValueError('retired stamp changed original create binding')
    if floors[0] == floors[1] and pair[0].get('last_retired') != pair[1].get('last_retired'):
        raise ValueError('same slot frontier has different terminal stamps')
    if all(floor >= window for floor in floors):
        return dict(status='retired_terminal_slot', identity=create_identity,
                    center_retired_through=floors[0], host_retired_through=floors[1])
    records = [value for value in center['sandboxes'] if value['create']['binding'] == create_identity['binding']]
    if len(records) > 1:
        raise ValueError('ambiguous original retained create')
    if not records:
        return None
    record = records[0]
    if original_create_identity(record) != create_identity:
        raise ValueError('original create binding changed')
    if record['released'] is not True or not record['operations']:
        return None
    latest = record['operations'][-1]
    if (type(latest) is not dict or set(latest) != CENTER_OPERATION_FIELDS
            or latest['unloaded'] is not None):
        raise ValueError('complete current original terminal operation required')
    response = latest['terminal']
    if response is None:
        return None
    if not _terminal_shape(response):
        raise ValueError('complete typed original terminal response required')
    if response['status'] != 'cleaned':
        return None
    if not _same_session(response, create_identity):
        raise ValueError('original terminal response changed create binding')
    intent = response['request']
    latest_identity = {key: latest[key] for key in ('binding', 'center', 'retention', 'accepted_version')}
    if (not _control_shape(latest_identity)
            or latest['binding'] != intent['request']['binding']
            or latest['accepted_version'] != intent['accepted_version']
            or latest['center'] != intent['center'] or latest['retention'] != intent['retention']
            or latest['operation'] != {'kind': 'destroy'}):
        raise ValueError('original terminal operation differs from its result')
    rows = [value for value in host['operations'] if value['current'] is True
            and value['binding'] == intent['request']['binding']]
    if len(rows) != 1:
        return None
    if not _binding_shape(rows[0]['binding']) or not _counter(rows[0]['accepted_version']):
        raise ValueError('typed original host operation required')
    confirmations = (record['terminal_confirmation'], rows[0].get('confirmed_terminal'))
    for confirmation in confirmations:
        if confirmation is not None and not _terminal_shape(confirmation):
            raise ValueError('complete typed original confirmation required')
    if (any(confirmation != response for confirmation in confirmations)
            or rows[0]['accepted_version'] != intent['accepted_version']):
        return None
    return dict(status='cleaned', identity=create_identity, response=response)


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
        if (not _control_shape(identity) or identity['accepted_version'] != 1
                or identity['binding']['node']['deployment_id'] != deployment
                or identity['binding']['key']['owner'] != str(owner)
                or not re.fullmatch(r'sandbox-[1-9][0-9]*', identity['binding']['session'])
                or (sandbox_id is not None and identity['binding']['session'] != sandbox_id)):
            raise RuntimeError('original runtime creation identity mismatch')
        outcome.update(sandbox_id=identity['binding']['session'], create_identity=identity)
        records = [record for record in center['sandboxes']
                   if record['create']['binding'] == identity['binding']]
        if len(records) > 1:
            raise RuntimeError('ambiguous original runtime create record')
        if records:
            record = records[0]
            if find_runtime_create(center, metadata, deployment=deployment) != identity:
                raise RuntimeError('original runtime create binding changed')
            if record['private'] or record['create']['resources'] != expected_resources:
                raise RuntimeError('original runtime creation resource binding changed')
            if record['phase'] == 'creating' and not record['released']:
                outcome['pending_create_observed'] = True
        host = read_host(identity['binding']['node'])
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
                status = delete(identity['binding']['session'])
                outcome.update(delete_http_status=status, delete_acknowledged=status in (200, 204))
                if status not in (200, 204, 404):
                    raise RuntimeError('original runtime deletion rejected')
            except OSError as error:
                outcome['delete_error_type'] = type(error).__name__
        sleep(min(.5, max(0, deadline - monotonic())))
    raise TimeoutError('original identity or confirmed terminal remains unknown; retain trial')
