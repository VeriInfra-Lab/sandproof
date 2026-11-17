"""Finite E8 consumer of #792's actual Memory sync gate and original End.

Destroy reception can commit while Full owns the execution guard. This does
not imply a terminal fence or source Stop has already executed. The original
Cleanup-success/pre-SQL cut supplies the later stable terminal observation.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import copy
import hashlib
import json
from pathlib import Path
import sqlite3
import time

from _internal_fault_relay import accepted_request, _fields, _u64, _opaque
from _center_host_services import local_binding
from lifecycle_unload_capture_faults import CaptureHandoffProbe, SYNC_PHASES, _AwaitTerminal
from lifecycle_unload_runtime import (checked_source_records, continued_capture_question,
    checked_file, gate_publish, material_source_charge, original_json, original_process, pidfd_exited, remaining)
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path
from tools.template_build_guest import equal


class SyncDestroyProbe(CaptureHandoffProbe):
    def __init__(self, runtime, client, physical_running, stage, gate_dir):
        if stage not in SYNC_PHASES:
            raise ValueError('one actual Memory sync boundary required')
        super().__init__(runtime, client, physical_running, stage, gate_dir)
        self.running_acceptance = copy.deepcopy(client.current_acceptance)
        self.original_entry = self.held = self.cleanup_cut = None
        self.command_publish_attempted = False
        self.incomplete_material_paths = None

    def _fault_environment(self):
        # Both controls already belong to the original root Host allowlist.
        return dict(super()._fault_environment(), SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION='cleanup_vm')

    def after_acceptance(self, accepted, deadline):
        result = super().after_acceptance(accepted, deadline)
        entry, _, _ = self._read(min(deadline, self.deadline))
        accepted_request(entry['intent'], self.runtime.services.host_configs[0]['inbox']['node'])
        self.original_entry = copy.deepcopy(entry)
        self._capture_resources(self.record)
        self.slot_gate = self._slot_gate()
        store = Path(self.runtime.captures[0]['layout_parent'])/'s'
        self.incomplete_material_paths = [str(store/(prefix+self.question['capture_key']))
                                          for prefix in ('staging-', 'capture-', 'completed-')]
        return result

    def _slot_gate(self):
        native = self.runtime.services.native_pools[0]
        position = self.prior['context']['position']
        root = Path(native['node_state_dir'])
        record = original_json(root/'host-slots'/f'{position}.json', 16384)
        _fields(record, 'position helper_gate value')
        gate = _fields(record['helper_gate'], 'path device inode max_data_routes_per_slot')
        path = root/'helper-gates'/f'{position}.lock'
        facts = checked_file(path, 16*1024**2)
        for key in ('device', 'inode', 'max_data_routes_per_slot'):
            _u64(gate[key], positive=key!='device')
        if (not equal(record['position'], position) or record['value'] is None
                or not equal(record['value']['context'], self.prior['context'])
                or gate['path'] != str(path) or (gate['device'], gate['inode']) != (facts['device'], facts['inode'])):
            raise ValueError('original protected slot context and exact helper gate required')
        return dict(path=str(path), gate=copy.deepcopy(gate), file=facts)

    def _registered(self, conn, action, capacity):
        _fields(action, 'identity command')
        identity = _fields(action['identity'], 'protocol_version owner sandbox operation epoch generation logical_action attempt resource')
        for key in ('protocol_version', 'epoch', 'generation', 'attempt'):
            _u64(identity[key])
        for key in ('owner', 'sandbox', 'operation', 'logical_action'):
            _opaque(identity[key])
        logical = identity['logical_action']; number = logical.removeprefix('effect-')
        if (not number.isascii() or not number.isdecimal() or not 0 < int(number) < 2**63
                or logical != 'effect-'+str(int(number))):
            raise ValueError('original canonical logical action registration required')
        size = conn.execute('SELECT length(logical_json) FROM controller_effect_keys WHERE id=?', (int(number),)).fetchone()
        if size is None or not 0 < size[0] <= capacity['max_sql_value_bytes']:
            raise ValueError('bounded original registered command required')
        raw, = conn.execute('SELECT logical_json FROM controller_effect_keys WHERE id=?', (int(number),)).fetchone()
        if not equal(json.loads(raw, object_pairs_hook=unique_keys), action['command']['logical']):
            raise ValueError('complete original logical action differs from its registration')

    def _snapshot(self, deadline):
        """All inbox, native, actions and source facts share one SQL snapshot."""
        native = self.runtime.services.native_pools[0]
        database = private_path(native['database'])
        before = database.stat()
        capacity = self.runtime.services.host_configs[0]['native']['maintenance']['capacity']
        with closing(sqlite3.connect(database.as_uri()+'?mode=ro', uri=True,
                                      timeout=min(2, remaining(deadline)))) as conn:
            conn.execute('BEGIN')
            size = conn.execute('SELECT length(state_json),reserve_bytes FROM controller_lifecycle_inbox WHERE id=1').fetchone()
            if size is None or not 0 < size[0] <= size[1] <= min(16*1024**2, capacity['max_sql_value_bytes']):
                raise ValueError('bounded original Host inbox required')
            raw, checksum = conn.execute('SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1').fetchone()
            if hashlib.sha256(raw).digest() != checksum:
                raise ValueError('original Host inbox checksum changed')
            old, terminal, local = self._terminal_entries_for(
                json.loads(raw, object_pairs_hook=unique_keys), self.original_entry)
            accepted_request(terminal['intent'], self.runtime.services.host_configs[0]['inbox']['node'])
            size = conn.execute('SELECT length(state_blob) FROM sandboxes WHERE sandbox_id=?', (self.config['sandbox'],)).fetchone()
            if size is None or not 0 < size[0] <= capacity['max_sql_value_bytes']:
                raise ValueError('bounded original native row required')
            raw, owner = conn.execute('SELECT state_blob,owner_id FROM sandboxes WHERE sandbox_id=?', (self.config['sandbox'],)).fetchone()
            state = json.loads(raw, object_pairs_hook=unique_keys)
            if owner != self.action['identity']['owner'] or state['owner'] != local['owner']:
                raise ValueError('original native owner changed')
            sources = checked_source_records(conn, self.runtime.services.host_configs[0], deadline)
            old_actions = self._terminal_outbox(conn, local, old['local_operation'], capacity)
            operation = 'op-'+native['namespace']+'-'+str(local['sandbox'])+'-'+str(terminal['local_operation'])
            count, = conn.execute('SELECT count(*) FROM outbox WHERE sandbox_id=? AND operation_id=?',
                                  (self.config['sandbox'], operation)).fetchone()
            # Reception admits Kill under the same writer while Full still
            # owns ActiveSandbox. No terminal action need be scheduled yet.
            terminal_actions = ([] if count == 0 else
                                self._terminal_outbox(conn, local, terminal['local_operation'], capacity))
            action_ids = []
            for action, _, _ in old_actions + terminal_actions:
                self._registered(conn, action, capacity)
                rows = conn.execute('SELECT id,action_json FROM outbox WHERE sandbox_id=? AND logical_action_id=? AND attempt=?',
                    (action['identity']['sandbox'], action['identity']['logical_action'], action['identity']['attempt'])).fetchall()
                if (len(rows) != 1 or type(rows[0][0]) is not int or not 0 < rows[0][0] < 2**63
                        or not equal(json.loads(rows[0][1], object_pairs_hook=unique_keys), action)):
                    raise ValueError('one exact original outbox identity/result row required')
                action_ids.append((rows[0][0], action))
            count, largest = conn.execute("SELECT count(*),max(length(action_json)) FROM outbox WHERE dispatch_state='dispatched'").fetchone()
            if count > capacity['max_sql_rows'] or (largest is not None and largest > capacity['max_sql_value_bytes']):
                raise ValueError('bounded global pending actions required')
            cleanups = []
            for raw, in conn.execute("SELECT action_json FROM outbox WHERE dispatch_state='dispatched' ORDER BY id"):
                action = json.loads(raw, object_pairs_hook=unique_keys)
                if 'CleanupVm' in action['command']['logical']['command']:
                    cleanups.append(action)
        after = database.stat()
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise ValueError('original SQLite replaced during terminal snapshot')
        return dict(original=old, terminal=terminal, local=local, native=state, sources=sources,
                    old_actions=old_actions, terminal_actions=terminal_actions, global_cleanups=cleanups,
                    action_ids=action_ids)

    def _source(self, snapshot):
        if not equal(snapshot['original'], self.original_entry):
            raise ValueError('complete original Unload entry changed')
        record = continued_capture_question(self.question, self.accepted, snapshot['sources'], self.prior['source'])
        if (not equal(record['work'], self.record['work'])
                or any(not equal(record['capture'][name], self.record['capture'][name])
                       for name in self.record['capture'] if name != 'end')
                or material_source_charge(record) != self.sync_reserved_bytes
                or self.sync_reserved_bytes <= 0):
            raise ValueError('complete original Full and positive material responsibility changed')
        self._capture_resources(record)
        rows = snapshot['old_actions'] + snapshot['terminal_actions']
        seals = [row for row in rows if 'SealSnapshot' in row[0]['command']['logical']['command']]
        if len(seals) != 1 or not equal(seals[0][0], self.action):
            raise ValueError('one unchanged original Seal required')
        if any(any(kind in row[0]['command']['logical']['command']
                   for kind in ('CommitSnapshot', 'PrepareVm', 'ActivateVm')) for row in rows):
            raise ValueError('terminal Full must not commit, prepare or activate a replacement')
        return record

    def _capture_resources(self, record):
        # controller::identity::resource maps Seal to Snapshot, while the
        # work requirement, terminal Stop and cleanup name the source VM.
        fields = _fields(self.action['command']['logical']['command']['SealSnapshot'],
                         'sandbox epoch generation vm workspace snapshot manifest')
        for value in fields.values():
            _u64(value)
        namespace = self.runtime.services.native_pools[0]['namespace']
        vm = dict(kind='vm', id='vm-'+namespace+'-'+str(fields['vm']))
        snapshot = dict(kind='snapshot', id='snapshot-'+namespace+'-'+str(fields['snapshot']))
        if (not equal(self.action['identity']['resource'], snapshot)
                or vm['id'] != self.prior['source']['resource_id']
                or not equal(record['work']['requirement']['resource'], vm)
                or not equal(record['capture']['full']['pause']['source_resource'], vm)):
            raise ValueError('original Snapshot Seal and distinct source VM resource relation required')
        return vm

    def _closed(self, snapshot):
        native = snapshot['native']; op = native['current_operation']
        for key in ('owner', 'generation', 'epoch'):
            _u64(native[key])
        if (native['owner'] != self.prior['local']['owner'] or native['terminal'] is not True
                or native['desired'] != 'Closed' or native['effective'] != 'Closed'
                or native['generation'] != self.action['identity']['generation']
                or native['epoch'] <= self.action['identity']['epoch'] or op is None
                or op['kind'] != 'Kill' or op['id'] != snapshot['terminal']['local_operation']
                or op['epoch'] != native['epoch']):
            raise ValueError('exact current native Kill and closed authority required')
        _u64(op['id']); _u64(op['epoch'])
        return native

    def _held_facts(self, snapshot):
        self._closed(snapshot)
        record = self._source(snapshot)
        if (not equal(snapshot['native']['current_vm'], self.prior['context']['binding']['vm'])
                or not equal(snapshot['native']['net_slot'], self.prior['context']['binding']['network'])):
            raise ValueError('held Full must retain its original native VM and network slot')
        if (record['capture']['completed'] is not None or record['capture']['end'] is not None
                or self.host.poll() is not None or pidfd_exited(self.worker_pidfd)
                or not original_process(self.question['worker']['process'])
                or pidfd_exited(self.pidfd) or not original_process(self.prior['source'])):
            raise ValueError('original live Full worker and source required at the held sync cut')
        seal = [row for row in snapshot['old_actions'] if equal(row[0], self.action)]
        if len(seal) != 1 or seal[0][1:] != ('dispatched', None):
            raise ValueError('original Seal receipt must remain pending at the held cut')
        origin = record['work']['anchor']['execution_origin']
        binding = origin['request']['binding']
        accepted_request(origin, self.runtime.services.host_configs[0]['inbox']['node'])
        if not equal(dict(session=binding['session'], key=binding['key'], accepted_version=origin['accepted_version']), self.running_acceptance):
            raise ValueError('complete current Resume acceptance must be the original execution origin')
        backing = [row for row in snapshot['sources'] if row['restore'] is not None
                   and equal(row['restore']['intent'], origin)]
        if (origin['request']['operation']['kind'] != 'resume' or len(backing) != 1
                or backing[0]['restore']['material_use'] is not True
                or material_source_charge(backing[0]) <= 0
                or not equal(backing[0]['capture']['completed'], origin['request']['operation']['manifest_digest'])):
            raise ValueError('original Resume backing pin and material charge required')
        resume = origin['request']['operation']
        previous = backing[0]['capture']['full']['pause']['binding']
        if (self.original_entry['intent']['request']['operation']['source_incarnation'] != resume['target_incarnation']
                or any(not equal(resume[key], previous[key]) for key in ('source_incarnation', 'capture'))):
            raise ValueError('exact Resume source/target and both original capture bindings required')
        if not equal(self.runtime._materials(backing[0]['capture']['full'], backing[0]['capture']), self.prior['running_backing']):
            raise ValueError('original backing material bytes changed')
        if not equal(self._sync_material(record['capture']['full']['memory_bytes']), self.sync_material):
            raise ValueError('original unsealed Memory changed before gate consumption')
        if not equal(self._slot_gate(), self.slot_gate):
            raise ValueError('original protected helper gate changed while Full was held')
        self._gate_unchanged()
        return dict(host_destroy_received=True, full_worker_still_live=True,
                    original_seal_sql_ack_pending=True, backing_pin_retained=True,
                    full_charge=material_source_charge(record), backing_charge=material_source_charge(backing[0]),
                    terminal_fence_inferred=False, public_completion=False, capacity_released=False)

    @staticmethod
    def _digest(value):
        if type(value) is not list or len(value) != 32 or any(type(b) is not int or not 0 <= b <= 255 for b in value):
            raise ValueError('actual bounded 32-byte binding required; no digest authenticity inferred')

    def _end(self, record, stop, action_ids):
        end = _fields(record['capture']['end'], 'state stop observation prior')
        seal = self.action['command']['logical']['command']['SealSnapshot']
        epoch = stop['identity']['epoch']
        result_ids = [number for number, action in action_ids if equal(action, stop)]
        if len(result_ids) != 1:
            raise ValueError('one actual successful Stop outbox result row required')
        coverage = dict(operation=dict(sandbox=seal['sandbox'], operation=self.terminal_entry['local_operation']),
                        epoch=epoch, attempt=stop['identity']['attempt'], result=result_ids[0])
        if end['state'] != 'terminated' or not equal(end['stop'], coverage):
            raise ValueError('actual original terminal drain and complete Stop coverage required')
        observation = _fields(end['observation'], 'state work_binding full_work_binding seal_action stop_action source worker host_slot helper_gate_device helper_gate_inode')
        self._digest(observation['work_binding'])
        _u64(observation['helper_gate_device']); _u64(observation['helper_gate_inode'])
        if (observation['state'] != 'stopped_and_drained'
                or not equal(checked_file(self.slot_gate['path'], 16*1024**2), self.slot_gate['file'])
                or (observation['helper_gate_device'], observation['helper_gate_inode']) !=
                   (self.slot_gate['file']['device'], self.slot_gate['file']['inode'])
                or not equal(observation['full_work_binding'], self.question['full_work_binding'])
                or not equal(observation['seal_action'], self.action) or not equal(observation['stop_action'], stop)
                or not equal(observation['source'], self.question['source'])
                or not equal(observation['worker'], self.question['worker'])
                or not equal(observation['host_slot'], self.prior['context'])):
            raise ValueError('complete original source/worker/Full/slot drain observation differs')
        prior = _fields(end['prior'], 'state observation')
        returned = _fields(prior['observation'], 'action full_work_binding worker source outcome')
        outcome = _fields(returned['outcome'], 'kind http_status')
        if (prior['state'] != 'worker_ended' or outcome['kind'] != 'returned'
                or type(outcome['http_status']) is not int or not 200 <= outcome['http_status'] <= 599
                or any(not equal(returned[name], self.question[name])
                       for name in ('action', 'full_work_binding', 'worker', 'source'))):
            raise ValueError('actual original Returned End required; never infer NoRequest or success')
        if not pidfd_exited(self.worker_pidfd) or not pidfd_exited(self.pidfd):
            raise ValueError('positive original worker and source pidfd exit required')
        return copy.deepcopy(end)

    def _cut_facts(self, snapshot):
        record = self._source(snapshot)
        native = self._closed(snapshot)
        self.terminal_entry = snapshot['terminal']
        commands = self.action['command']['logical']['command']['SealSnapshot']
        vm = {key: commands[key] for key in ('sandbox', 'generation', 'vm')}
        rows = snapshot['terminal_actions']
        def one(kind):
            selected = [row for row in rows if kind in row[0]['command']['logical']['command']]
            if len(selected) != 1:
                raise ValueError('one original terminal '+kind+' required')
            return selected[0]
        fence, stop, cleanup = one('InstallFence'), one('StopVm'), one('CleanupVm')
        for row, kind, fields in ((fence, 'InstallFence', dict(sandbox=vm['sandbox'], epoch=native['epoch'], terminal=True)),
                                 (stop, 'StopVm', dict(vm, epoch=native['epoch'])), (cleanup, 'CleanupVm', vm)):
            action = row[0]
            _fields(action, 'identity command')
            identity = _fields(action['identity'], 'protocol_version owner sandbox operation epoch generation logical_action attempt resource')
            _fields(action['command'], 'attempt logical'); _u64(action['command']['attempt'])
            _fields(action['command']['logical'], 'command')
            for key in ('protocol_version', 'epoch', 'generation', 'attempt'):
                _u64(identity[key])
            _opaque(identity['logical_action'])
            expected_identity = dict(self.action['identity'], epoch=native['epoch'],
                operation='op-'+self.runtime.services.native_pools[0]['namespace']+'-'+str(vm['sandbox'])+'-'+str(self.terminal_entry['local_operation']),
                logical_action=identity['logical_action'], attempt=action['command']['attempt'],
                resource=dict(kind='sandbox', id=self.config['sandbox']) if kind=='InstallFence' else record['work']['requirement']['resource'])
            if (not equal(action['command']['logical']['command'], {kind:fields})
                    or not equal(identity, expected_identity)):
                raise ValueError('terminal action differs from original complete source command')
        observed_fence = self._success_observation(fence, 'Fence')
        expected_fence = dict(epoch=native['epoch'], terminal=True)
        if (not equal(observed_fence, expected_fence)
                or not equal(original_json(Path(self.runtime.services.native_pools[0]['node_state_dir']) /
                    'fences' / (self.config['sandbox']+'.json'), 16384), expected_fence)):
            raise ValueError('actual protected terminal fence required')
        source_stop = self._success_observation(stop, 'Stopped')
        if (not equal(source_stop, dict(pid=self.prior['source']['pid'], start_ticks=self.prior['source']['start_time_ticks']))
                or not equal(stop[0]['identity']['resource'], record['work']['requirement']['resource'])
                or not equal(cleanup[0]['identity']['resource'], record['work']['requirement']['resource'])
                or cleanup[1:] != ('dispatched', None)
                or not equal(snapshot['global_cleanups'], [cleanup[0]])):
            raise ValueError('same-source Stop and unique global pending Cleanup required to bind exit86')
        return dict(original_end=self._end(record, stop[0], snapshot['action_ids']), original_stop=copy.deepcopy(stop[0]),
                    cleanup=copy.deepcopy(cleanup[0]), cleanup_sql_ack_pending=True,
                    original_source_exited=True, original_worker_exited=True,
                    public_completion=False, capacity_released=False)

    def _audit_cut(self):
        audit = self.runtime.services.processes.audit('host-0')
        node = self.runtime.services.host_configs[0]['inbox']['node']
        local = local_binding(audit, self.accepted['session'], expected_node=node)
        rows = [row for row in audit['operations'] if row['current'] is True
                and row['binding']['session'] == self.accepted['session']]
        if (len(rows) != 1 or not equal(rows[0]['binding'], self.terminal_entry['intent']['request']['binding'])
                or rows[0]['accepted_version'] != self.terminal_accepted['accepted_version']
                or rows[0]['terminal'] is not True
                or rows[0]['local_operation'] != self.terminal_entry['local_operation']
                or any(local[key] != self.prior['local'][key] for key in ('sandbox', 'owner'))):
            raise ValueError('original Rust audit differs from complete current Destroy/mapping')
        return dict(original_rust_audit_accepted=True, current_destroy_binding_matched=True,
                    physical_truth_inferred_from_json=False)

    def recover_after_destroy(self, accepted, deadline, *, observe_peer):
        if (self.original_entry is None or self.terminal_accepted is not None
                or self.command_publish_attempted or self.command_sent or self.recovered):
            raise ValueError('one held original Full and public Destroy required')
        deadline = min(deadline, self.deadline)
        self._bind_next_destroy(accepted)
        while True:
            try:
                self.held = self._held_facts(self._snapshot(deadline))
                break
            except _AwaitTerminal:
                if self.host.poll() is not None:
                    raise ValueError('original Host exited before receiving Destroy')
                self._gate_unchanged()
                time.sleep(min(.05, remaining(deadline)))
        peer = observe_peer()
        self._held_facts(self._snapshot(deadline))
        command = dict(schema_version=3, run_identity=self.config['run_identity'],
            observation_sha256=self.question_digest, full_work_binding=self.question['full_work_binding'], command='fail')
        remaining(deadline)
        self._gate_unchanged()
        self.command_publish_attempted = True
        gate_publish(self.fd, 'command.json', command)
        self.command_sent = True
        self._save_file('command.json')
        if self.host.wait(timeout=remaining(deadline)) != 86:
            raise ValueError('original Host did not reach Cleanup-success/pre-SQL exit86')
        self._configuration()
        consumed, _ = self._save_file('consumed.json')
        if not equal(consumed, command):
            raise ValueError('original sync fail command was not consumed exactly')
        self._gate_unchanged()
        self.cleanup_cut = self._cut_facts(self._snapshot(deadline))
        # Original Rust audit reopens and validates saved source/native coverage;
        # Python neither recreates its encoding digest nor grants a drain permit.
        audited = self._audit_cut()
        self.client.assert_terminal_unload_replay(phase='terminating')
        process = self.runtime.services.processes.start('host-0')
        if process is self.host or process.pid == self.host.pid:
            raise ValueError('fresh original manager required after cleanup cut')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        self._configuration(); remaining(deadline)
        self.recovered = True
        return dict(held=self.held, peer=peer, consumed_command=command, terminal=self.cleanup_cut, audit=audited,
                    original_recovery_resumed=True, public_completion=False)

    def finish(self, deadline):
        raise ValueError('terminal sync branch requires the exact public Destroy and original End')

    def cleanup(self):
        # These paths were not adopted into public Unloaded. Wait for the
        # original final settlement oracle, rather than strengthening Cleaned
        # into an instantaneous material-retirement guarantee.
        if (not self.recovered or not self.runtime.gc_complete or not self.runtime.physical_complete
                or self.incomplete_material_paths is None):
            raise ValueError('original recovery and final product material/resource settlement required')
        if any(Path(name).exists() or Path(name).is_symlink() for name in self.incomplete_material_paths):
            raise ValueError('original incomplete Full paths remain after product settlement')
        return dict(super().cleanup(), incomplete_material_paths_absent=copy.deepcopy(self.incomplete_material_paths))

    def terminal_failure_state(self):
        return dict(original_acceptance=copy.deepcopy(self.accepted), destroy_acceptance=copy.deepcopy(self.terminal_accepted),
                    held=copy.deepcopy(self.held), command_publish_attempted=self.command_publish_attempted,
                    command_sent=self.command_sent, cleanup_cut=copy.deepcopy(self.cleanup_cut),
                    incomplete_material_paths=copy.deepcopy(self.incomplete_material_paths),
                    public_completion=False, capacity_released=False,
                    action='none; preserve original gate, accepted requests and unknown material/resource duties')
