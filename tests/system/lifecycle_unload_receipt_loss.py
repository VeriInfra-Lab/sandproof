"""E13 first-Unload receipt-loss observer; no VM runner or success synthesizer.

Inputs are the original relay observation, public acceptance, native center
audits and Processes observations. Call order is hold -> center_stopped ->
discarded -> restarted -> recovered. Only the center may restart in that
interval. Take the restarted process sample BEFORE a subsequent planned native
center audit stops its writer. Host audits that restart Host belong outside it.

This checks decoded evidence, not physical capture integrity, authorization,
JSON/TLS correctness, Redis durability under power loss or complete E13.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy

from _internal_fault_relay import (
    _fields, _u64, accepted_request, same_json, unloaded_observation,
)


def observe_survivors(processes):
    """Use original procfs/cgroup observer with the owned center already dead.

    Never stop/audit/restart Host to obtain this sample. Redis and Host must be
    the actual registered children, with original argv, credentials and inode.
    """
    from _center_host_services import process_facts
    if (set(processes.bindings) != {'center', 'redis', 'host-0'}
            or set(processes.children) != set(processes.bindings)
            or processes.children['center'].poll() is None):
        raise ValueError('one original cohort with stopped center required')
    result = {}
    for name in ('redis', 'host-0'):
        role = processes.bindings[name]
        argv = [str(role['binary']), str(role['config'])] if name == 'redis' else None
        result[name] = process_facts(processes.children[name], role['binary'],
            role['config'], processes.record, uid=role['uid'], gid=role['gid'], argv=argv)
    return result


def _processes(value, names):
    _fields(value, names)
    for role in value.values():
        _fields(role, 'pid start_time_ticks uid gid executable')
        for name in ('pid', 'start_time_ticks', 'uid', 'gid'):
            _u64(role[name], positive=name in ('pid', 'start_time_ticks'))
        for fact in _fields(role['executable'], 'device inode').values():
            _u64(fact)
    if len({row['pid'] for row in value.values()}) != len(value):
        raise ValueError('service roles must have distinct actual processes')
    return copy.deepcopy(value)


def _intent(row):
    _fields(row, 'center binding retention accepted_version acceptance_time operation running create_refusal terminal unloaded')
    return dict(request=dict(binding=row['binding'], operation=row['operation']),
                center=row['center'], retention=row['retention'],
                accepted_version=row['accepted_version'])


class UnloadRestartObserver:
    """Correlate one original Create -> Unload, with every envelope field pinned.

    create_identity and lifetime come from pre-fault native running evidence.
    Public acceptance alone contains neither the capture nor retention binding.
    Saved material metadata is compared in full; its digest is no file proof.
    """
    def __init__(self, *, node, acceptance, create_identity, lifetime):
        _fields(acceptance, 'session key accepted_version')
        _fields(create_identity, 'binding center retention accepted_version')
        _u64(acceptance['accepted_version'], positive=True)
        _u64(create_identity['accepted_version'], positive=True)
        _fields(lifetime, 'started_at_ms end_at_ms max_end_at_ms startup_deadline_ms')
        for value in lifetime.values():
            _u64(value)
        if not (lifetime['started_at_ms'] <= lifetime['startup_deadline_ms']
                and lifetime['started_at_ms'] < lifetime['end_at_ms'] <= lifetime['max_end_at_ms']):
            raise ValueError('original live lifetime required')
        self.node = copy.deepcopy(node)
        self.acceptance = copy.deepcopy(acceptance)
        self.create_identity = copy.deepcopy(create_identity)
        self.lifetime = copy.deepcopy(lifetime)
        self.phase = 'initial'

    def _phase(self, expected):
        if self.phase != expected:
            raise ValueError('receipt-loss observations out of order')

    def hold(self, observation, processes):
        self._phase('initial')
        request = observation['request']
        if not unloaded_observation(request, observation, self.node):
            raise ValueError('only a real completed Unload opens the fault window')
        binding = request['request']['binding']
        acceptance = dict(session=binding['session'], key=binding['key'],
                          accepted_version=request['accepted_version'])
        facts = observation['outcome']['facts']
        if (not same_json(self.acceptance, acceptance)
                or not same_json(self.lifetime, {name: facts[name] for name in self.lifetime})
                or request['accepted_version'] != 2
                or self.create_identity['accepted_version'] != 1):
            raise ValueError('first Unload changed original acceptance or lifetime')
        self.before = _processes(processes, 'center redis host-0')
        self.observation = copy.deepcopy(observation)
        self.phase = 'held'

    def _record(self, audit):
        _fields(audit, 'schema_version versions deployment revision sandbox_highwater operation_highwater reserved_resources nodes retention_lanes sandboxes')
        if (type(audit['schema_version']) is not int or audit['schema_version'] != 3
                or audit['deployment'] != self.node['deployment_id']
                or type(audit['sandboxes']) is not list):
            raise ValueError('original native center audit required')
        request = self.observation['request']
        binding = request['request']['binding']
        rows = [row for row in audit['sandboxes']
                if row['create']['binding']['session'] == binding['session']]
        keys = [operation for row in audit['sandboxes'] for operation in row['operations']
                if same_json(operation['binding']['key'], binding['key'])]
        if len(rows) != 1 or len(keys) != 1:
            raise ValueError('original session and complete operation key must be unique')
        row = rows[0]
        _fields(row, 'create private operations terminating released phase last_running terminal_confirmation')
        _fields(row['create'], 'binding template_revision template_digest resources options_sha256 startup_deadline_ms timeout_ms max_lifetime_ms metadata_sha256')
        if (len(row['operations']) != 2 or row['private'] is not False
                or row['terminating'] is not False or row['released'] is not False
                or row['terminal_confirmation'] is not None):
            raise ValueError('original visible first-Unload obligation required')
        first, current = row['operations']
        original = _intent(first)
        original_identity = dict(binding=original['request']['binding'],
            center=original['center'], retention=original['retention'],
            accepted_version=original['accepted_version'])
        if (not same_json(original_identity, self.create_identity)
                or not same_json(row['create']['binding'], original_identity['binding'])
                or not same_json(first['operation'], dict(kind='create', target_incarnation=request['request']['operation']['source_incarnation']))
                or not same_json(original_identity['binding']['node'], binding['node'])
                or original_identity['binding']['session'] != binding['session']
                or original_identity['binding']['key']['owner'] != binding['key']['owner']
                or not same_json(_intent(current), request)):
            raise ValueError('original Create/Unload association changed')
        accepted_request(_intent(current), self.node)
        running = _fields(row['last_running'], 'started_at_ms end_at_ms max_end_at_ms startup_deadline_ms observed_at_ms')
        for value in running.values():
            _u64(value)
        if (not same_json({name: running[name] for name in self.lifetime}, self.lifetime)
                or not same_json(first['running'], running)
                or not running['started_at_ms'] <= running['observed_at_ms'] < running['end_at_ms']
                or any(current[name] is not None for name in ('running', 'terminal', 'create_refusal'))):
            raise ValueError('original running history or Unload result shape changed')
        for value in _fields(audit['reserved_resources'], 'instances cpu memory_bytes disk_bytes').values():
            _u64(value, positive=True)
        return row

    def center_stopped(self, audit, survivors):
        self._phase('held')
        survivors = _processes(survivors, 'redis host-0')
        if not same_json(survivors, {name: self.before[name] for name in survivors}):
            raise ValueError('Host or Redis restarted inside the center-only fault window')
        row = self._record(audit)
        if row['phase'] != 'unloading' or row['operations'][-1]['unloaded'] is not None:
            raise ValueError('center must retain the accepted Unload without its reply')
        self.pending = copy.deepcopy(row)
        # Revision and sender/retention progress may advance on recovery. These
        # original resource/identity facts must not change or refund early.
        names = 'schema_version versions deployment sandbox_highwater operation_highwater reserved_resources nodes'.split()
        self.center_facts = copy.deepcopy({name: audit[name] for name in names})
        self.phase = 'stopped'

    def discarded(self, evidence):
        self._phase('stopped')
        _fields(evidence, 'observation held dropped response_attempts')
        held = _u64(evidence['held'], positive=True)
        if (_u64(evidence['dropped']) != held or _u64(evidence['response_attempts']) != 0
                or not same_json(evidence['observation'], self.observation)):
            raise ValueError('original completed replies must all be explicitly discarded')
        self.loss = copy.deepcopy(evidence)
        self.phase = 'discarded'

    def restarted(self, processes):
        self._phase('discarded')
        after = _processes(processes, 'center redis host-0')
        for name in ('host-0', 'redis'):
            if not same_json(after[name], self.before[name]):
                raise ValueError('only the center may restart')
        old, new = self.before['center'], after['center']
        if (new['start_time_ticks'] < old['start_time_ticks']
                or (new['pid'], new['start_time_ticks']) == (old['pid'], old['start_time_ticks'])
                or any(not same_json(new[name], old[name]) for name in ('uid', 'gid', 'executable'))):
            raise ValueError('new center process must use the original executable and identity')
        self.after = after
        self.phase = 'restarted'

    def recovered(self, audit):
        self._phase('restarted')
        row = self._record(audit)
        expected = copy.deepcopy(self.pending)
        expected['phase'] = 'unloaded'
        expected['operations'][-1]['unloaded'] = self.observation
        if (not same_json(row, expected)
                or not same_json({name: audit[name] for name in self.center_facts}, self.center_facts)):
            raise ValueError('recovery changed the original obligation, result or reservation')
        self.phase = 'recovered'
        return copy.deepcopy(dict(scope='e13-first-unload-center-receipt-loss',
            loss=self.loss, before=self.before, after=self.after,
            create_identity=self.create_identity, lifetime=self.lifetime,
            acceptance=self.acceptance, observation=self.observation))
