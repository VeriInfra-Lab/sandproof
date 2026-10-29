"""Read-only retirement observations on the original lifecycle deployment.

The strict current SourceWork reader checks saved accounting and plan bindings.
Read-only observations do not provide an end certificate, close a pin or delete
materials. The explicit G7 probe restarts original managers only at the verified
cut. A filesystem absence never substitutes for Settled.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_runpy.run_path(str(_GuardPath(__file__).with_name('_assertions_required.py')))[
    'require_assertions']()

from contextlib import closing
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import stat
import time

from lifecycle_unload_runtime import (
    checked_source_records, gate_json, gate_publish, material_source_charge,
    original_process, remaining, validate_gate_ready,
)
from tools.system_acceptance import private_path
from tools.check_kvm_host import unique_keys


def read_records(runtime, deadline):
    """One read-only WAL snapshot; never use the manager-stopping audit helper."""
    return _read_snapshot(runtime, deadline)[0]


def _read_snapshot(runtime, deadline, accepted=None):
    database = private_path(runtime.services.native_pools[0]['database'])
    stamp = database.stat()
    identity = (stamp.st_dev, stamp.st_ino)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True,
            timeout=min(2, remaining(deadline)))) as connection:
        connection.execute('BEGIN')
        records = checked_source_records(connection, runtime.services.host_configs[0], deadline)
        successor = None
        if accepted is not None:
            header = connection.execute('SELECT length(state_json),reserve_bytes FROM '
                'controller_lifecycle_inbox WHERE id=1').fetchone()
            if header is None or not 0 < header[0] <= header[1] <= 16*1024**2:
                raise ValueError('bounded original lifecycle inbox required at retirement gate')
            raw,checksum = connection.execute('SELECT state_json,sha256 FROM '
                'controller_lifecycle_inbox WHERE id=1').fetchone()
            if hashlib.sha256(raw).digest() != checksum:
                raise ValueError('original lifecycle inbox checksum differs at retirement gate')
            inbox = json.loads(raw,object_pairs_hook=unique_keys)
            successor = accepted_successor(records,inbox,accepted,
                runtime.services.host_configs[0]['inbox']['node'])
    after = database.stat()
    if (after.st_dev, after.st_ino) != identity:
        raise ValueError('original SQLite replaced during material retirement observation')
    return records,successor


def accepted_successor(records,inbox,accepted,node):
    """Select B from original accepted SQL identity, without public completion."""
    sessions = [row for row in inbox['sessions'] if row['id'] == accepted['session']]
    if (len(sessions) != 1 or sessions[0]['terminal'] is not False
            or sessions[0]['current_version'] != accepted['accepted_version']):
        raise ValueError('original current nonterminal successor acceptance required')
    entries = [entry for entry in sessions[0]['entries']
        if entry['intent']['accepted_version'] == accepted['accepted_version']
        and entry['intent']['request']['binding']['key'] == accepted['key']]
    if len(entries) != 1 or entries[0]['local_operation'] is None:
        raise ValueError('one original admitted successor Unload required')
    intent = entries[0]['intent']
    request = intent['request']
    operation = request['operation']
    if (request['binding']['session'] != accepted['session']
            or request['binding']['node'] != node or operation['kind'] != 'unload'):
        raise ValueError('successor acceptance has a different original request')
    rows = [row for row in records if row['work']['anchor']['request'] == intent]
    if len(rows) != 1 or rows[0]['capture'] is None:
        raise ValueError('one original accepted successor Full responsibility required')
    successor = rows[0]
    capture = successor['capture']
    binding = capture['full']['pause']['binding']
    if (binding['operation'] != request['binding']
            or binding['source_incarnation'] != operation['source_incarnation']
            or binding['capture'] != operation['capture']
            or capture['operation'] != entries[0]['local_operation']
            or type(capture['completed']) is not list or len(capture['completed']) != 32
            or any(type(n) is not int or not 0 <= n <= 255 for n in capture['completed'])):
        raise ValueError('successor capture or complete manifest differs from original acceptance')
    return successor


def source_record(records, session, physical):
    facts = physical['facts']
    matches = [record for record in records
        if record['work']['anchor']['request']['request']['binding']['session'] == session
        and record['capture'] is not None
        and record['capture']['completed'] == facts['manifest_digest']
        and record['capture']['full']['pause']['binding']['capture'] == facts['capture']
        and record['capture']['full']['pause']['binding']['source_incarnation']
            == facts['source_incarnation']]
    if len(matches) != 1:
        raise ValueError('one retained original manifest-bound material row required')
    return matches[0]


def retirement_snapshot(records, *, session, original, successor, resume):
    """Select original rows; None means the exact old obligation is still held."""
    old = source_record(records, session, original)
    current = source_record(records, session, successor)
    if old is current or old['capture']['completed'] == current['capture']['completed']:
        raise ValueError('distinct original and successor material identities required')
    claim = old['restore']
    if (claim is None or claim['activation'] is None
            or claim['intent']['request']['binding']['session'] != session
            or claim['intent']['request']['binding']['key'] != resume['key']
            or claim['intent']['accepted_version'] != resume['accepted_version']):
        raise ValueError('original consumed Resume claim and activation required')
    if (current['retirement'] is not None or current['restore'] is not None
            or material_source_charge(current) <= 0):
        raise ValueError('complete current unclaimed successor must remain held and charged')
    retirement = old['retirement']
    charge = material_source_charge(old)
    if retirement is None or retirement['state'] == 'retiring':
        if charge != old['capture']['reserved_material_bytes']:
            raise ValueError('unsettled original retirement released its reservation')
        return None
    if retirement['state'] != 'settled' or claim['material_use'] is not False or charge != 0:
        raise ValueError('original material plan, use and reservation are not settled')
    held = [row for row in records
        if row['work']['anchor']['request']['request']['binding']['session'] == session
        and material_source_charge(row) > 0]
    if held != [current]:
        raise ValueError('completed old generations still retain material reservations')
    return dict(old=copy.deepcopy(old), successor=copy.deepcopy(current),
        held_material_bytes=material_source_charge(current),
        source_history_records=sum(row['work']['anchor']['request']['request']['binding'][
            'session'] == session for row in records))


def wait_settled(runtime, *, session, original, successor, resume, prior_running, deadline):
    """Wait for a persisted result, then independently check exact old paths."""
    while True:
        snapshot = retirement_snapshot(read_records(runtime, deadline), session=session,
            original=original, successor=successor, resume=resume)
        if snapshot is not None:
            break
        # This is a bounded observation interval, never a fault scheduling point.
        time.sleep(min(0.05, remaining(deadline)))
    if original_process(prior_running['source']):
        raise ValueError('original restored VMM still live after material settlement')
    release = snapshot['old']['material_release']
    if (release['producer'] is None or release['consumer'] is None
            or release['producer']['context'] != original['original_position']
            or release['consumer']['context'] != prior_running['context']
            or any(type(release[name]['input_sequence']) is not int
                or release[name]['input_sequence'] <= 0 for name in ('producer','consumer'))):
        raise ValueError('fixed original producer and restored consumer release witnesses differ')
    paths = [Path(value) for value in original['material_paths']]
    if any(path.exists() or path.is_symlink() for path in paths):
        raise ValueError('settled original material paths remain present')
    plan = snapshot['old']['retirement']['plan']
    return dict(snapshot=snapshot, receipt=dict(
        original_capture=original['facts']['capture'],
        successor_capture=successor['facts']['capture'],
        original_work_binding=copy.deepcopy(plan['work_binding']),
        original_plan_binding=copy.deepcopy(plan['plan_binding']),
        released_material_bytes=snapshot['old']['capture']['reserved_material_bytes'],
        held_material_bytes=snapshot['held_material_bytes'],
        original_material_paths_absent=[str(path) for path in paths],
        original_restored_vmm_exited=True, live_session=True,
        original_release_facts=copy.deepcopy(release),
        source_history_records=snapshot['source_history_records'],
        logical_settlement_not_physical_block_measurement=True))


def assert_replay_unchanged(runtime, before, *, session, original, successor, resume, deadline):
    after = retirement_snapshot(read_records(runtime, deadline), session=session,
        original=original, successor=successor, resume=resume)
    if after is None or after != before:
        raise ValueError('old Resume replay changed original claim, plan or current source')


def gate_material(record, capture_key):
    """Bind the actual opaque SQL plan; never regenerate a physical delete plan."""
    retirement = record['retirement']
    if retirement is None or retirement['state'] != 'retiring':
        raise ValueError('original SQL Retiring required at the deletion gate')
    plan = retirement['plan']
    raw = bytes(plan['bytes'])
    if (hashlib.sha256(raw).digest() != bytes(plan['plan_binding'])
            or material_source_charge(record) != record['capture']['reserved_material_bytes']):
        raise ValueError('original raw plan and full Retiring charge required')
    return dict(sandbox=record['work']['anchor']['local']['sandbox'],
        generation=record['work']['anchor']['native']['generation'], capture_key=capture_key,
        work_binding=bytes(plan['work_binding']).hex(),
        plan_binding=hashlib.sha256(raw).hexdigest(), plan_bytes=len(raw))


def validate_retirement_gate(value, *, run_identity, expected, source_resource):
    if (type(value) is not dict or set(value) != {
            'schema_version','run_identity','phase','material','source_resource'}
            or type(value['schema_version']) is not int or value['schema_version'] != 1
            or value['run_identity'] != run_identity
            or value['phase'] != 'material-deleted-before-settlement'
            or type(value['material']) is not dict or value['material'] != expected
            or set(value['material']) != {
                'sandbox','generation','capture_key','work_binding','plan_binding','plan_bytes'}
            or any(type(value['material'][name]) is not int for name in (
                'sandbox','generation','plan_bytes'))
            or value['source_resource'] != source_resource):
        raise ValueError('exact original material/work/plan deletion gate required')
    return value


class RetirementRestartProbe:
    """One original post-delete/pre-settlement gate; no second VM or collector."""

    def __init__(self, runtime, session, original, deadline):
        self.runtime, self.session = runtime, session
        self.original = copy.deepcopy(original)
        self.deadline, self.fd = deadline, None
        self.files, self.recovered = {}, False

    def _configuration(self):
        from _center_host_services import role_facts
        role = self.runtime.services.processes.bindings['host-0']
        if role_facts(Path(role['config']).parent,role['config'],uid=role['uid'],gid=role['gid']) != self.runtime.configs['host-0']:
            raise ValueError('original Host configuration changed at material retirement gate')

    def arm(self, timeout_ms):
        runtime = self.runtime
        run = runtime.lease.identity
        record = source_record(read_records(runtime,self.deadline),self.session,self.original)
        key = Path(self.original['material_paths'][0]).name
        sandbox = record['work']['anchor']['local']['sandbox']
        generation = record['work']['anchor']['native']['generation']
        if (self.fd is not None or type(run) is not str or re.fullmatch('[0-9a-f]{64}',run) is None
                or re.fullmatch('capture-[0-9a-f]{64}',key) is None
                or any(type(n) is not int or not 0 < n < 2**64 for n in (sandbox,generation))
                or type(timeout_ms) is not int or not 1000 <= timeout_ms <= 120000):
            raise ValueError('one bounded original material retirement target required')
        self.config = dict(schema_version=1,run_identity=run,sandbox=sandbox,
            generation=generation,capture_key=key,timeout_ms=timeout_ms)
        parent = private_path(runtime.quota['deployment_control'],directory=True)
        self.path = parent/'material-retirement-gate'
        self.path.mkdir(mode=0o700)
        self.fd = os.open(self.path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
        self.identity = os.fstat(self.fd)
        gate_publish(self.fd,'config.json',self.config)
        self._save('config.json')
        self.env = {'SANDPROOF_TEST_MATERIAL_RETIREMENT_GATE':str(self.path)}
        self._configuration()
        runtime.services.processes.stop('host-0')
        self.host = runtime.services.processes.start('host-0',env=self.env)
        runtime.services.wait_ready(timeout=min(remaining(self.deadline),900),for_admission=False)
        return dict(armed=True,capture_key=key,public_completion=False)

    def _save(self,name):
        value,digest,identity = gate_json(self.path/name,
            16384 if name=='observation.json' else 1024,directory_fd=self.fd)
        self.files[name] = (digest,identity)
        return value,digest

    def _unchanged(self):
        current,held = self.path.lstat(),os.fstat(self.fd)
        identity = (self.identity.st_dev,self.identity.st_ino)
        if (not stat.S_ISDIR(current.st_mode) or current.st_uid != 0
                or stat.S_IMODE(current.st_mode) != 0o700
                or (current.st_dev,current.st_ino) != identity
                or (held.st_dev,held.st_ino) != identity
                or set(os.listdir(self.fd)) - {
                    'config.json','observation.json','ready.json','consumed.json',
                    'ready-stage.json','command.json','command-stage.json'}):
            raise ValueError('original private material retirement gate changed')
        for name,expected in self.files.items():
            _,digest,stamp = gate_json(self.path/name,
                16384 if name=='observation.json' else 1024,directory_fd=self.fd)
            if (digest,stamp) != expected:
                raise ValueError('original sealed material retirement gate file changed')

    def _wait_ready(self):
        while True:
            # The producer's timeout starts at its actual post-delete wait.
            # Preparing B can take longer; arm must not spend that budget.
            remaining(self.deadline)
            self._unchanged()
            if self.runtime.services.processes.children['host-0'] is not self.host or self.host.poll() is not None:
                raise ValueError('original manager exited before the material deletion gate')
            try:
                ready,_ = self._save('ready.json')
            except FileNotFoundError:
                time.sleep(min(0.05,remaining(self.deadline)))
                continue
            return ready

    def restart_after_delete(self, successor_acceptance):
        runtime = self.runtime
        ready = self._wait_ready()
        observation,digest = self._save('observation.json')
        validate_gate_ready(ready,digest)
        records,successor = _read_snapshot(runtime,self.deadline,successor_acceptance)
        original = source_record(records,self.session,self.original)
        material = gate_material(original,self.config['capture_key'])
        validate_retirement_gate(observation,run_identity=runtime.lease.identity,
            expected=material,source_resource=original['capture']['full']['pause']['source_resource'])
        consumed,_ = self._save('consumed.json')
        if (type(consumed) is not dict or set(consumed) != {
                'schema_version','run_identity','observation_sha256','material'}
                or type(consumed['schema_version']) is not int or consumed['schema_version'] != 1
                or consumed != dict(schema_version=1,run_identity=runtime.lease.identity,
                    observation_sha256=digest,material=material)):
            raise ValueError('original durable one-shot material gate consumption required')
        reason = original['retirement']['plan']['closure']['reason']
        if (reason['kind'] != 'superseded'
                or reason['successor_work_binding'] != successor['work']['requirement']['work_binding']
                or reason['manifest_digest'] != successor['capture']['completed']):
            raise ValueError('original frozen closure does not name this accepted successor')
        if successor['retirement'] is not None or successor['restore'] is not None or material_source_charge(successor) <= 0:
            raise ValueError('current successor not held while old deletion is unsettled')
        # Full byte/manifest validation is read-only and leaves no source FD open.
        complete = runtime._materials(successor['capture']['full'],successor['capture'])
        if any(Path(path).exists() or Path(path).is_symlink() for path in self.original['material_paths']):
            raise ValueError('real original delete has not completed at the selected gate')
        self._unchanged()
        self._configuration()
        restart = runtime.restart_managers(self.deadline,host_env=self.env)
        self._unchanged()
        # Do not let the later physical audit stop/restart Host before this
        # same-environment retry has actually consumed and settled its plan.
        while True:
            reopened = source_record(read_records(runtime,self.deadline),self.session,self.original)
            state = reopened['retirement']
            if (state is None or state['plan'] != original['retirement']['plan']
                    or reopened['restore'] != original['restore']
                    or reopened['material_release'] != original['material_release']):
                raise ValueError('manager restart replaced the original retirement responsibility')
            if state['state'] == 'settled':
                if material_source_charge(reopened) != 0:
                    raise ValueError('original restarted plan has not settled its reservation')
                break
            if state['state'] != 'retiring' or material_source_charge(reopened) != material_source_charge(original):
                raise ValueError('restarted original retirement changed its held reservation')
            self._unchanged()
            time.sleep(min(0.05,remaining(self.deadline)))
        self._unchanged()
        self.recovered = True
        return dict(phase=observation['phase'],material=material,
            retiring_material_bytes=material_source_charge(original),
            successor_manifest=complete['manifest'],manager_restart=restart,
            successor_accepted_version=successor_acceptance['accepted_version'],
            successor_capture=successor['capture']['full']['pause']['binding']['capture'],
            public_unloaded_not_inferred_from_gate=True,
            same_gate_consumed_before_restart=True,
            same_plan_settled_after_restart=True,settled_material_bytes=0)

    def cleanup(self):
        if not self.recovered or not self.runtime.gc_complete or not self.runtime.physical_complete:
            raise ValueError('material retirement gate retained until original product settlement')
        self._unchanged()
        if set(os.listdir(self.fd)) != set(self.files):
            raise ValueError('unobserved material retirement gate file remains')
        for name in self.files:
            os.unlink(name,dir_fd=self.fd)
        os.fsync(self.fd)
        self.close()
        self.path.rmdir()
        return 'exact-observed-material-retirement-gate-retired-after-product-settlement'

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
