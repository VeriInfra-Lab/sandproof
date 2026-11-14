"""One E9 late duplicate of original StopVm/CleanupVm after product slot reuse.

Only the existing root-only Node probe executes old actions. Its refusal report
is paired with independent SQL/kernel/metadata observations and guest I/O. This
does not claim arbitrary late unlink, CleanupSlot, or the complete E9 matrix.
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
from pathlib import Path
import re
import sqlite3
import time

from _center_host_services import role_facts
from lifecycle_late_material_receipt import SlotOccupantObserver, _bounded_json
from lifecycle_unload_capture_faults import CaptureHandoffProbe, terminal_outbox
from lifecycle_unload_runtime import original_process, remaining
from server_bounded_churn import (
    live_identity, node_configuration, tree_records, validate_stale_report, write_private,
)
from server_process_crash import process_start_ticks
from tools.check_kvm_host import artifact_fingerprint, unique_keys
from tools.ci_e2e_prepare import bounded_json_command
from tools.system_acceptance import private_path


def approved_probe(args):
    """Require the helper from the same prepared Cargo target as the Host."""
    path, digest = getattr(args, 'stale_probe', None), getattr(args, 'stale_probe_sha256', None)
    if (path is None or type(digest) is not str or re.fullmatch('[0-9a-f]{64}', digest) is None
            or Path(path) != Path(args.host).parent/'examples/stale_host_actions'):
        raise ValueError('exact prepared stale_host_actions executable and SHA256 required')
    return artifact_fingerprint(dict(path=str(path), sha256=digest), executable=True)


def accepted_entry(inbox, accepted, prior, node, *, terminal):
    sessions = [s for s in inbox['sessions'] if s['id'] == accepted['session']]
    if (len(sessions) != 1 or sessions[0]['current_version'] != accepted['accepted_version']
            or sessions[0]['terminal'] is not terminal):
        raise ValueError('exact current original Host session/version required')
    session = sessions[0]
    if any(session['local'][k] != prior['local'][k] for k in ('sandbox', 'owner')):
        raise ValueError('original public/native mapping changed')
    entries = [e for e in session['entries'] if e['intent']['accepted_version'] == accepted['accepted_version']
        and e['intent']['request']['binding']['key'] == accepted['key']]
    if len(entries) != 1 or entries[0]['local_operation'] is None:
        raise ValueError('one original admitted Host operation required')
    entry = entries[0]; binding = entry['intent']['request']['binding']
    if (binding['session'] != accepted['session'] or binding['node'] != node
            or str(prior['local']['owner']) != accepted['key']['owner']):
        raise ValueError('original complete session/node/owner binding differs')
    return entry, session['local']


def original_actions(rows, prior, native, resource):
    """Select the two real current-generation terminal commands, never synthesize."""
    binding = prior['context']['binding']
    selected = {}
    for row in rows:
        command = row[0]['command']['logical']['command']
        for kind in ('StopVm', 'CleanupVm'):
            if kind in command:
                if kind in selected or set(command) != {kind}:
                    raise ValueError('one original Stop and Cleanup action required')
                selected[kind] = row
    if set(selected) != {'StopVm', 'CleanupVm'}:
        raise ValueError('both actual old stop/delete actions required before journal retirement')
    expected = dict(sandbox=prior['local']['sandbox'], generation=binding['generation'], vm=binding['vm'])
    # ResourceRef comes from the original persisted Prepare, never a guessed id.
    stop, cleanup = selected['StopVm'], selected['CleanupVm']
    for row, kind in ((stop, 'StopVm'), (cleanup, 'CleanupVm')):
        action = row[0]; identity = action['identity']
        want = dict(expected, epoch=native['epoch']) if kind == 'StopVm' else expected
        if (action['command']['logical']['command'] != {kind: want}
                or identity['epoch'] != native['epoch']
                or identity['resource'] != resource):
            raise ValueError('old terminal action has a foreign VM/generation/resource/epoch')
    if (cleanup[1:] != ('dispatched', None)
            or CaptureHandoffProbe._success_observation(stop, 'Stopped') != dict(
                pid=prior['source']['pid'], start_ticks=prior['source']['start_time_ticks'])):
        raise ValueError('original successful Stop and unacknowledged Cleanup cut required')
    return [copy.deepcopy(stop[0]), copy.deepcopy(cleanup[0])]


class LateHostActionsProbe:
    def __init__(self, runtime, client, prior, resume, helper, deadline):
        if prior['session'] != client.session or resume != client.current_acceptance:
            raise ValueError('original restored execution and Resume acceptance required')
        self.runtime, self.client, self.deadline = runtime, client, deadline
        self.prior, self.resume, self.helper = copy.deepcopy((prior, resume, helper))
        self.host = self.entry = self.actions = None
        self.recovered = self.replayed = False
        self.native = copy.deepcopy(runtime.services.native_pools[0])
        self.host_config = copy.deepcopy(runtime.services.host_configs[0])

    def _configuration(self):
        services = self.runtime.services
        role = services.processes.bindings['host-0']
        if (services.native_pools[0] != self.native or services.host_configs[0] != self.host_config
                or role_facts(Path(role['config']).parent, role['config'],
                    uid=role['uid'], gid=role['gid']) != self.runtime.configs['host-0']):
            raise ValueError('original Host deployment/configuration changed')

    def _read(self, accepted, *, terminal):
        self._configuration()
        database = private_path(self.native['database'])
        before = database.stat()
        capacity = self.host_config['native']['maintenance']['capacity']
        with closing(sqlite3.connect(database.as_uri()+'?mode=ro', uri=True,
                timeout=min(2, remaining(self.deadline)))) as conn:
            conn.execute('BEGIN')
            header = conn.execute('SELECT length(state_json),reserve_bytes FROM '
                'controller_lifecycle_inbox WHERE id=1').fetchone()
            if (header is None or not 0 < header[0] <= header[1]
                    <= min(capacity['max_sql_value_bytes'], 16*1024**2)):
                raise ValueError('bounded original Host inbox required')
            raw, checksum = conn.execute('SELECT state_json,sha256 FROM '
                'controller_lifecycle_inbox WHERE id=1').fetchone()
            if hashlib.sha256(raw).digest() != checksum:
                raise ValueError('original inbox checksum differs')
            entry, local = accepted_entry(json.loads(raw, object_pairs_hook=unique_keys),
                accepted, self.prior, self.host_config['inbox']['node'], terminal=terminal)
            sandbox = 'sb-'+self.native['namespace']+'-'+str(local['sandbox'])
            native = _bounded_json(conn, 'sandboxes', 'state_blob', 'sandbox_id', sandbox,
                capacity['max_sql_value_bytes'])
            if native['owner'] != local['owner'] or native['generation'] != self.prior['context']['binding']['generation']:
                raise ValueError('original terminal native generation/owner differs')
            if terminal:
                operation = native['current_operation']
                if (native['terminal'] is not True or operation is None or operation['kind'] != 'Kill'
                        or operation['id'] != entry['local_operation'] or operation['epoch'] != native['epoch']):
                    raise ValueError('original admitted terminal Kill is no longer current')
            binding = entry['intent']['request']['binding']
            expected_identity = dict(owner='tenant-'+str(local['owner']),
                protocol_version=binding['protocol_version'], generation=native['generation'])
            rows = terminal_outbox(self.runtime, conn, local, entry['local_operation'], capacity, expected_identity)
        after = database.stat()
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise ValueError('original database replaced during action capture')
        return copy.deepcopy(entry), native, rows

    def arm(self):
        if self.host is not None:
            raise ValueError('one original Cleanup cut required')
        self.entry, _, rows = self._read(self.resume, terminal=False)
        if self.entry['intent']['request']['operation']['kind'] != 'resume':
            raise ValueError('original admitted Resume required')
        prepared = [row[0] for row in rows if 'PrepareVm' in row[0]['command']['logical']['command']]
        binding = self.prior['context']['binding']
        if (len(prepared) != 1 or any(prepared[0]['command']['logical']['command']['PrepareVm'][k] != v
                for k,v in dict(sandbox=binding['sandbox'], generation=binding['generation'], vm=binding['vm']).items())):
            raise ValueError('one original persisted Prepare for the restored execution required')
        self.resource = copy.deepcopy(prepared[0]['identity']['resource'])
        self.runtime.services.processes.stop('host-0')
        self._configuration()
        self.host = self.runtime.services.processes.start('host-0',
            env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'cleanup_vm'})
        self.runtime.services.wait_ready(timeout=min(remaining(self.deadline), 900), for_admission=False)
        return dict(cut='cleanup_vm-success-before-SQL-receipt', preparatory_host_restart=True)

    def capture(self, accepted):
        if (self.host is None or self.actions is not None or accepted != self.client.current_acceptance
                or accepted['session'] != self.resume['session']
                or accepted['accepted_version'] != self.resume['accepted_version']+1
                or self.client.destroy_request != dict(window=accepted['key']['window'],
                    sequence=accepted['key']['sequence'], expected_version=self.resume['accepted_version'])):
            raise ValueError('one exact successor public Destroy required')
        while self.host.poll() is None:
            time.sleep(min(.02, remaining(self.deadline)))
        if self.host.poll() != 86 or self.runtime.services.processes.children['host-0'] is not self.host:
            raise ValueError('original Host did not reach successful Cleanup pre-receipt exit86')
        entry, native, rows = self._read(accepted, terminal=True)
        expected = copy.deepcopy(self.entry['intent'])
        expected['accepted_version'] = accepted['accepted_version']
        expected['request']['binding'].update(key=copy.deepcopy(accepted['key']),
            expected=dict(state='version', version=self.resume['accepted_version']))
        expected['request']['operation'] = dict(kind='destroy')
        if entry['intent'] != expected or accepted['key'] == self.resume['key']:
            raise ValueError('complete original Destroy intent differs from the accepted successor')
        actions = original_actions(rows, self.prior, native, self.resource)
        if original_process(self.prior['source']):
            raise ValueError('original stopped VM still live')
        self.actions = actions
        return dict(actual_commands=['StopVm', 'CleanupVm'], original_stop_receipted=True,
            original_cleanup_executed_receipt_pending=True, original_vmm_exited=True,
            actions_sha256=hashlib.sha256(json.dumps(actions, sort_keys=True).encode()).hexdigest())

    def recover(self):
        if self.actions is None or self.recovered or self.host.poll() != 86:
            raise ValueError('one captured original Cleanup cut required before recovery')
        self._configuration()
        self.runtime.services.processes.start('host-0')
        self.runtime.services.wait_ready(timeout=min(remaining(self.deadline), 900), for_admission=False)
        self.recovered = True

    def replay(self, occupant, guest_check):
        old, new = self.prior['context'], occupant['context']
        if (not self.recovered or self.replayed or old['position'] != new['position']
                or old['binding']['owner'] == new['binding']['owner']
                or old['binding']['sandbox'] == new['binding']['sandbox']
                or new['expected_previous'] != old['binding']['vm']
                or new['binding']['vm'] <= old['binding']['vm']):
            raise ValueError('one actual immediate reuse by the other owner required')
        self._configuration()
        self.runtime.services.processes.stop('host-0')
        self._configuration()
        observer = SlotOccupantObserver(self.runtime, occupant=occupant, deadline=self.deadline)
        observer.freeze()
        roots = (self.native['node_state_dir'], Path(self.native['chroot_base'])/'.claims')
        before = tree_records(roots)
        identity = live_identity(self.native, new, process_start_ticks)
        directory = private_path(self.runtime.root, directory=True)
        config, actions = directory/'late-stop-delete-node.json', directory/'late-stop-delete-actions.json'
        write_private(config, node_configuration(self.native))
        write_private(actions, dict(version=1, old_context=old, actions=self.actions))
        # Same approved executable, rechecked immediately before any old command.
        stamp = artifact_fingerprint(dict(path=self.helper['path'], sha256=self.helper['sha256']), executable=True)
        if stamp != self.helper:
            raise ValueError('approved old-action helper identity changed')
        self.replayed = True
        result = bounded_json_command([stamp['path'], str(config), str(actions)],
            {'PATH':'/usr/bin:/bin', 'LANG':'C'}, timeout=min(30, remaining(self.deadline)), max_bytes=65536)
        validate_stale_report(result, 2)
        if (tree_records(roots) != before
                or live_identity(self.native, new, process_start_ticks) != identity):
            raise ValueError('late stop/delete mutated metadata or current occupant resources')
        observation = observer.verify()
        self._configuration()
        self.runtime.services.processes.start('host-0')
        self.runtime.services.wait_ready(timeout=min(remaining(self.deadline), 900), for_admission=False)
        guest = guest_check()  # Original B public RAM/disk/process/credential round trip.
        observer.verify()
        return dict(probe=result, helper_sha256=stamp['sha256'], occupant=observation,
            metadata_and_claims_unchanged=True, same_kernel_identity=True, guest=guest,
            scope='two original StopVm/CleanupVm duplicates; not arbitrary unlink or CleanupSlot')


def exercise_late_stop_delete(runtime, client, control, saved, deadline, report, helper):
    report['phase'] = 'late-actions-original-resume'
    accepted = client.resume(saved)
    resumed = client.wait_running()
    running = runtime.record_running(client.session, deadline, resume_acceptance=accepted)
    report['late_actions_resumed_workload'] = client.assert_resumed_workload(resumed)
    client.operation_evidence_completed()
    probe = LateHostActionsProbe(runtime, client, running, accepted, helper, deadline)
    report['phase'] = 'late-actions-arm-original-cleanup-cut'
    report['late_actions_arm'] = probe.arm()
    report['destroy'] = client.destroy(resumed)
    client.assert_data_closed()
    report['phase'] = 'late-actions-capture-original-stop-cleanup'
    report['late_actions_capture'] = probe.capture(report['destroy'])
    probe.recover()
    client.wait_cleaned()
    report['final_resources'] = runtime.destroy_evidence(client.session, deadline)
    client.operation_evidence_completed()
    report['old_session_settlement'] = runtime.settlement_evidence(deadline)
    report['phase'] = 'late-actions-other-owner-slot-reuse'
    control.create(runtime.create_parameters(owner=2))
    current = control.wait_running()
    control.assert_owner_isolation()
    occupant = runtime.record_running(control.session, deadline)
    report['slot_occupant_workload'] = control.run_public_workload(current)
    control.operation_evidence_completed()
    report['slot_reuse'] = dict(occupant_session=control.session, position=occupant['context']['position'])
    report['phase'] = 'late-actions-replay-old-stop-cleanup'
    report['late_stop_delete'] = probe.replay(occupant, lambda:control.assert_running_workload(current))
    report['phase'] = 'late-actions-other-owner-destroy-gc'
    from lifecycle_unload_scenario import finish_slot_occupant
    finish_slot_occupant(control, current, runtime, deadline, report)
    report['phase'] = 'quota-settlement'
    started = time.monotonic()
    report['settlement'] = runtime.settlement_evidence(deadline)
    finished = time.monotonic()
    report['settlement_timing'] = dict(check_seconds=finished-started,
        final_destroy_send_to_settled_seconds=finished-control.timings[-1]['sent_monotonic'],
        completion_condition='both original public Destroy/GC plus zero reservations/material charge/backing pins')
    report['acceptance_cases'] = dict(E9='two original terminal StopVm/CleanupVm commands replayed after '
        'other-owner immediate slot reuse; refusal plus unchanged SQL/kernel/claims and same guest I/O',
        other_cases='one late-duplicate representative; not arbitrary late deletion or complete E9/E12/E13')
    report['phase'], report['complete'] = 'accepted-scenario', True
