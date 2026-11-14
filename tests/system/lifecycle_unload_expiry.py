"""Actual expiry observations; no Destroy request, clock rewrite or ledger edit.

The existing Rust audit reads the original Redis envelope while its writer is
stopped. Only background product work may create the terminal request before
that audit; public queries are deferred until the durable request is observed.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import copy
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time

from lifecycle_unload_capture_faults import CaptureHandoffProbe
from lifecycle_unload_runtime import (checked_source_records, material_source_charge,
    open_original_candidate_pidfd, original_json, original_process, pidfd_exited, remaining)
from lifecycle_unload_retirement import read_records
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path


def now_ms():
    return time.time_ns() // 1_000_000


def require_before_expiry(lifetime):
    if now_ms() >= lifetime['end_at_ms']:
        raise ValueError('expiry occurred before the selected physical boundary; scenario not reached')


def wait_clock(lifetime, deadline):
    while True:
        left = remaining(deadline)
        delay = (lifetime['end_at_ms'] - now_ms()) / 1000
        if delay <= 0:
            return
        time.sleep(min(.1,left,delay))


def expiry_acceptance(audit, session, resume, lifetime, sampled_now):
    """Check actual Rust audit fields, never invent a public Destroy key/time."""
    if sampled_now < lifetime['end_at_ms']:
        raise ValueError('real persisted deadline has not arrived')
    rows = [r for r in audit['sandboxes'] if r['create']['binding']['session'] == session]
    if len(rows) != 1:
        raise ValueError('original session missing from center authority')
    row = rows[0]; operations = row['operations']
    if len(operations) == 3 and operations[-1]['operation']['kind'] == 'resume' and not row['terminating']:
        return None
    if (len(operations) != 4 or [o['operation']['kind'] for o in operations] !=
            ['create', 'unload', 'resume', 'destroy'] or not row['terminating'] or row['released']):
        raise ValueError('expiry must append only one terminal request while material is held')
    prior, terminal = operations[-2:]
    binding = terminal['binding']; key = binding['key']
    if (prior['binding']['key'] != resume['key'] or prior['accepted_version'] != resume['accepted_version']
            or binding['session'] != session or binding['node'] != prior['binding']['node']
            or binding['protocol_version'] != prior['binding']['protocol_version']
            or binding['expected'] != {'state':'version','version':resume['accepted_version']}
            or terminal['accepted_version'] != resume['accepted_version'] + 1
            or terminal['center'] != prior['center'] or terminal['retention'] != prior['retention']
            or key['window'] != 2 or type(key['sequence']) is not int or key['sequence'] <= 0
            or any(key[k] != resume['key'][k] for k in ('owner','deployment'))
            or key == resume['key'] or terminal['acceptance_time'] != {'kind':'unsampled'}
            or any(row['last_running'][k] != v for k,v in lifetime.items())
            or row['create']['timeout_ms'] != lifetime['end_at_ms'] - lifetime['started_at_ms']):
        raise ValueError('automatic terminal request or original lifetime differs')
    return dict(session=session, key=copy.deepcopy(key), accepted_version=terminal['accepted_version'])


def read_host(runtime, session, deadline):
    database = private_path(runtime.services.native_pools[0]['database'])
    before = database.stat()
    with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,
                                 timeout=min(2,remaining(deadline)))) as conn:
        conn.execute('BEGIN')
        size = conn.execute('SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1').fetchone()
        if size is None or not 0 < size[0] <= size[1] <= 16*1024**2:
            raise ValueError('bounded original Host inbox required')
        raw, checksum = conn.execute('SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1').fetchone()
        if hashlib.sha256(raw).digest() != checksum:
            raise ValueError('Host inbox checksum changed')
        inbox = json.loads(raw,object_pairs_hook=unique_keys)
        matches = [r for r in inbox['sessions'] if r['id'] == session]
        if len(matches) != 1:
            raise ValueError('one original Host session required')
    after = database.stat()
    if (before.st_dev,before.st_ino) != (after.st_dev,after.st_ino):
        raise ValueError('original database replaced')
    return matches[0]


def observe_center_expiry(runtime, client, resume, lifetime, deadline, *, host_delivered):
    """No public query until original background expiry is durable."""
    if getattr(client,'destroy_request',None) is not None:
        raise ValueError('explicit Destroy cannot certify automatic expiry')
    wait_clock(lifetime, deadline)
    if host_delivered:
        while True:
            host = read_host(runtime, client.session, deadline)
            if host['terminal']:
                break
            time.sleep(min(.05,remaining(deadline)))
    polls = 0
    while True:
        remaining(deadline)
        runtime.services.processes.stop('center')
        try:
            audit = runtime.services.processes.audit('center')
            accepted = expiry_acceptance(audit,client.session,resume,lifetime,now_ms())
        finally:
            runtime.services.processes.start('center')
        polls += 1
        if accepted is not None:
            break
        # Explicit polling of durable facts, not a sleep used as proof of expiry.
        time.sleep(min(1,remaining(deadline)))
    if host_delivered:
        runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
    client.current_acceptance = copy.deepcopy(accepted)  # Observe the actual server key; allocate nothing.
    return dict(acceptance=accepted,end_at_ms=lifetime['end_at_ms'],observed_at_ms=now_ms(),
                observed_monotonic=time.monotonic(),center_audits=polls,
                explicit_destroy_sent=False,public_query_before_durable_expiry=False,
                acceptance_time='unsampled: no invented expiry acceptance timestamp')


def wait_expired_cleaned(client, deadline):
    """Automatic terminal work is not a fake client Destroy timing sample."""
    while True:
        remaining(deadline)
        status, raw = client.call('GET',client.path(),key=client.owner_key)
        if status == 503:
            time.sleep(min(.1,remaining(deadline)));continue
        if status != 200:
            raise ValueError('expiry query did not return original authoritative state')
        view = json.loads(raw,object_pairs_hook=unique_keys)
        client.check_view(view,client.current_acceptance)
        if view['phase'] == 'cleaned':
            return view
        if view['phase'] != 'terminating':
            raise ValueError('automatic terminal decision was lost or revived')
        time.sleep(min(.1,remaining(deadline)))


class ExpiryPrepareProbe(CaptureHandoffProbe):
    """Reuse the exact pre-marker barrier; bind an observed internal expiry key."""
    def _wait_native_expiry(self,deadline):
        """Center is down: observe the original Host's own deadline compensation."""
        pool=self.runtime.services.native_pools[0]
        database=private_path(pool['database'])
        before=database.stat()
        capacity=self.runtime.services.host_configs[0]['native']['maintenance']['capacity']
        while True:
            with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,
                                         timeout=min(2,remaining(deadline)))) as conn:
                conn.execute('BEGIN')
                size=conn.execute('SELECT length(state_blob) FROM sandboxes WHERE sandbox_id=?',
                                  (self.config['sandbox'],)).fetchone()
                if size is None or not 0<size[0]<=capacity['max_sql_value_bytes']:
                    raise ValueError('bounded original native expiry row required')
                raw,owner=conn.execute('SELECT state_blob,owner_id FROM sandboxes WHERE sandbox_id=?',
                                      (self.config['sandbox'],)).fetchone()
                state=json.loads(raw,object_pairs_hook=unique_keys)
            after=database.stat()
            if ((before.st_dev,before.st_ino)!=(after.st_dev,after.st_ino)
                    or owner!=self.action['identity']['owner']
                    or state['generation']!=self.action['identity']['generation']
                    or state['epoch']!=self.action['identity']['epoch']):
                raise ValueError('native expiry changed original database, owner or candidate')
            op=state['current_operation']
            expected_operation='op-'+pool['namespace']+'-'+str(self.prior['local']['sandbox'])+'-'+str(op['id']) if op else None
            if (op is None or op['kind']!='Resume' or op['epoch']!=state['epoch']
                    or expected_operation!=self.action['identity']['operation']):
                raise ValueError('Host expiry must retain the original Resume')
            if state['terminal']:
                if (op['status']!={'Compensating':'DeadlineExceeded'} or state['phase']!='Revoking'
                        or state['desired']!='Closed' or state['effective']!='Closed'
                        or state['current_vm'] is not None or state['net_slot'] is not None):
                    raise ValueError('original Resume has not entered closed deadline compensation')
                return dict(operation=op,epoch=state['epoch'],generation=state['generation'],
                            terminal=True,center_stopped=True)
            time.sleep(min(.05,remaining(deadline)))

    def _terminal_inventory(self,snapshot):
        if snapshot['original']['local_operation'] == snapshot['terminal']['local_operation']:
            if snapshot['resume_actions'] != snapshot['terminal_actions']:
                raise ValueError('two reads of the same original operation differ')
            rows = snapshot['resume_actions']
        else:
            rows = snapshot['resume_actions'] + snapshot['terminal_actions']
        prepared = [r for r in rows if 'PrepareVm' in r[0]['command']['logical']['command']]
        if len(prepared)!=1 or prepared[0][0]!=self.action:
            raise ValueError('original expiry operation has a missing or second Prepare')
        if any('ActivateVm' in r[0]['command']['logical']['command'] for r in rows):
            raise ValueError('expired LoadPaused candidate gained activation')
        if self._success_observation(prepared[0],'Prepared') is not None:
            fences = [r for r in rows if r[0]['command']['logical']['command'].get('InstallFence',{}).get('terminal') is True]
            if len(fences)!=1:
                raise ValueError('one actual terminal fence required at the cleanup cut')
            expected = dict(epoch=snapshot['native']['epoch'],terminal=True)
            action=fences[0][0]
            if (action['command']['logical']['command']['InstallFence'] != dict(
                    sandbox=snapshot['local']['sandbox'],**expected)
                    or action['identity']['epoch'] != expected['epoch']):
                raise ValueError('terminal fence names a different sandbox or epoch')
            if self._success_observation(fences[0],'Fence')!=expected:
                raise ValueError('terminal fence has no complete successful receipt')
            pool=self.runtime.services.native_pools[0]
            if original_json(Path(pool['node_state_dir'])/'fences'/(self.config['sandbox']+'.json'),16384)!=expected:
                raise ValueError('terminal fence file differs before cleanup settlement')
        return prepared[0]

    def _terminal_pause_barrier(self,snapshot):
        native=snapshot['native'];operation=native['current_operation']
        if operation is not None and operation['kind']=='Kill':
            qualification=dict(kind='current-kill',fence=super()._terminal_pause_barrier(snapshot))
        else:
            prepared=self._terminal_inventory(snapshot)
            if (operation is None or operation['kind']!='Resume'
                    or operation['status']!={'Compensating':'DeadlineExceeded'}
                    or operation['id']!=snapshot['original']['local_operation']
                    or operation['id']!=snapshot['terminal']['local_operation']
                    or operation['epoch']!=native['epoch'] or native['epoch']!=self.action['identity']['epoch']
                    or native['generation']!=self.action['identity']['generation']
                    or not native['terminal'] or native['phase']!='Revoking'
                    or native['desired']!='Closed' or native['effective']!='Closed'
                    or native['current_vm'] is not None or native['net_slot'] is not None
                    or prepared[1:]!=('dispatched',None)):
                raise ValueError('exact closed original Resume deadline compensation required')
            pool=self.runtime.services.native_pools[0]
            fence=original_json(Path(pool['node_state_dir'])/'fences'/(self.config['sandbox']+'.json'),16384)
            if fence not in (dict(epoch=native['epoch'],terminal=False),dict(epoch=native['epoch'],terminal=True)):
                raise ValueError('readonly recovery must retain the original exact fence')
            qualification=dict(kind='same-resume-deadline-compensation',fence=fence,
                scope='only clear the process observation fault; LoadPaused is not guest activation')
        records=[r for r in snapshot['sources'] if r['restore'] is not None
            and r['restore']['intent']==self.restore_entry['intent']]
        if (len(records)!=1 or records[0]['work']!=self.restore_record['work']
                or records[0]['capture']!=self.restore_record['capture']
                or records[0]['restore']!=self.restore_record['restore']
                or records[0]['retirement'] is not None
                or records[0]['restore']['activation'] is not None
                or records[0]['restore']['material_use'] is not True
                or material_source_charge(records[0])!=material_source_charge(self.restore_record)
                or material_source_charge(records[0])<=0):
            raise ValueError('expiry lost the original unknown loader material responsibility')
        material=self.runtime._materials(records[0]['capture']['full'],records[0]['capture'])
        qualification['original_material_files_verified']=len(material['materials'])==3
        return qualification

    def expire(self, resume, lifetime, deadline, *, observe_peer):
        deadline = min(deadline,self.deadline)
        require_before_expiry(lifetime)
        self._start_terminal_recovery(deadline)
        require_before_expiry(lifetime)
        # Hold only the center manager, not the clock or either VM. This makes
        # the Host deadline win deterministically; otherwise a center Kill can
        # win and never exercise the missing-revocation regression.
        self.runtime.services.processes.stop('center')
        try:
            wait_clock(lifetime,deadline)
            native_expiry=self._wait_native_expiry(deadline)
        finally:
            self.runtime.services.processes.start('center')
        expiry = observe_center_expiry(self.runtime,self.client,resume,lifetime,deadline,host_delivered=True)
        self.terminal_accepted = copy.deepcopy(expiry['acceptance'])
        # The inherited reader still checks the full original Resume/Destroy
        # intents, local mapping, native fence, original pin and positive charge.
        recovery = self._continue_terminal_recovery(deadline,observe_peer=observe_peer)
        recovery['automatic_terminal_reached_host'] = recovery.pop('public_destroy_reached_host')
        recovery['expiry_observation_authority'] = recovery.pop('terminal_fence_observed')
        recovery['native_deadline_compensation'] = native_expiry
        if recovery['expiry_observation_authority']['kind']!='same-resume-deadline-compensation':
            raise ValueError('center Kill substituted for the required Host deadline compensation')
        return dict(expiry=expiry,recovery=recovery)


class RunningExpiryProbe:
    """Hold the real Stop dispatch before its side effect with the original exit87 hook."""
    def __init__(self,runtime,client,running,resume,lifetime,deadline):
        self.runtime,self.client,self.running,self.resume,self.lifetime = runtime,client,running,resume,lifetime
        self.before = self._source(deadline)
        require_before_expiry(lifetime)
        self.pidfd = open_original_candidate_pidfd(running['source'])

    def arm(self,deadline):
        self.runtime.services.processes.stop('host-0')
        self.host = self.runtime.services.processes.start('host-0',env={'SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION':'stop_vm'})
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        require_before_expiry(self.lifetime)

    def _source(self,deadline):
        matches = [r for r in read_records(self.runtime,deadline) if r['restore'] is not None
            and r['restore']['intent']['request']['binding']['session'] == self.client.session
            and r['restore']['intent']['request']['binding']['key'] == self.resume['key']]
        if len(matches) != 1:
            raise ValueError('one original restored backing record required')
        r = matches[0]
        if (r['restore']['material_use'] is not True or r['restore']['activation'] is None
                or r['retirement'] is not None or material_source_charge(r) <= 0):
            raise ValueError('running backing lost original activation, pin or material charge')
        return r

    def held_after_expiry(self,deadline):
        wait_clock(self.lifetime,deadline)
        if self.host.wait(timeout=remaining(deadline)) != 87:
            raise ValueError('original manager did not exit before its Stop effect')
        if pidfd_exited(self.pidfd) or not original_process(self.running['source']):
            raise ValueError('live restored user disappeared before retained-material observation')
        source = self._source(deadline)
        if (source['capture'] != self.before['capture'] or source['restore'] != self.before['restore']
                or material_source_charge(source) != material_source_charge(self.before)):
            raise ValueError('expiry changed still-used source or material responsibility')
        pool = self.runtime.services.native_pools[0]
        native_id = 'sb-'+pool['namespace']+'-'+str(self.running['local']['sandbox'])
        capacity = self.runtime.services.host_configs[0]['native']['maintenance']['capacity']
        db = private_path(pool['database'])
        with closing(sqlite3.connect(db.as_uri()+'?mode=ro',uri=True,timeout=min(2,remaining(deadline)))) as conn:
            conn.execute('BEGIN')
            raw = conn.execute('SELECT state_blob FROM sandboxes WHERE sandbox_id=?',(native_id,)).fetchone()
            if raw is None or len(raw[0]) > capacity['max_sql_value_bytes']:
                raise ValueError('original bounded native row required')
            state = json.loads(raw[0],object_pairs_hook=unique_keys)
            op = state['current_operation']
            if not state['terminal'] or op is None or op['kind'] != 'Kill':
                raise ValueError('original expiry Kill not current')
            opid = 'op-'+pool['namespace']+'-'+str(self.running['local']['sandbox'])+'-'+str(op['id'])
            rows = conn.execute('SELECT action_json,dispatch_state,receipt_json FROM outbox WHERE sandbox_id=? AND operation_id=?',(native_id,opid)).fetchall()
            actions = [(json.loads(a,object_pairs_hook=unique_keys),s,r) for a,s,r in rows]
        stops = [r for r in actions if 'StopVm' in r[0]['command']['logical']['command']]
        vm = self.running['context']['binding']
        expected = dict(sandbox=self.running['local']['sandbox'],generation=vm['generation'],vm=vm['vm'],epoch=state['epoch'])
        if (len(stops)!=1 or stops[0][1:]!=('dispatched',None)
                or stops[0][0]['command']['logical']['command']['StopVm']!=expected
                or stops[0][0]['identity']['epoch']!=state['epoch']):
            raise ValueError('exit87 was not the exact original unacknowledged Stop')
        identity = stops[0][0]['identity']
        original = self.before['restore']['activation']['identity']
        if (identity['sandbox']!=native_id or identity['operation']!=opid
                or identity['generation']!=vm['generation']
                or identity['owner']!=original['owner']
                or identity['protocol_version']!=original['protocol_version']
                or identity['resource']!={'kind':'vm','id':self.running['source']['resource_id']}):
            raise ValueError('Stop has a foreign complete work identity')
        fence = original_json(Path(pool['node_state_dir'])/'fences'/(native_id+'.json'),16384)
        if fence!={'epoch':state['epoch'],'terminal':True}:
            raise ValueError('actual terminal fence not present before the held Stop')
        # Reuse the full original manifest/file verifier while the VMM is alive.
        physical = self.runtime._materials(source['capture']['full'],source['capture'])
        if any(physical[k]!=self.running['running_backing'][k] for k in ('materials','manifest','completion','material_paths')):
            raise ValueError('live backing changed or was deleted at expiry')
        if pidfd_exited(self.pidfd) or not original_process(self.running['source']):
            raise ValueError('original VMM exited during material observation')
        return dict(original_vmm_live=True,original_stop_dispatched_without_receipt=True,
                    original_material_files_unchanged=True,material_use_pin=True,
                    material_charge=material_source_charge(source),physical_exit=False)

    def recover(self,deadline):
        self.runtime.services.processes.start('host-0')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)

    def close(self):
        if self.pidfd is not None:
            os.close(self.pidfd);self.pidfd=None
