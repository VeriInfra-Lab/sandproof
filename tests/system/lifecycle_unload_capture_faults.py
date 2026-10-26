"""Finite consumers of original capture handoffs; never a VM supervisor.

Call arm only after record_running. During a held copy gate no helper may stop
or audit the Host. Public completion and product GC remain the caller's job.
An exit code, gate command or missing process alone never means Unloaded/Free.
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
import signal
from pathlib import Path
import sqlite3
import stat
import time

from _center_host_services import local_binding, role_facts
from lifecycle_unload_runtime import (
    checked_file, checked_source_records, continued_capture_question, gate_configuration,
    material_source_charge,
    gate_json, gate_publish, open_original_candidate_pidfd, original_json,
    original_process, pidfd_exited, remaining, validate_gate_ready,
)
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path

STAGES = {
    'freeze-receipt': ('freeze_vm', 'FreezeVm'),
    'copy-partial': (None, 'SealSnapshot'),
    'seal-receipt': ('seal_snapshot', 'SealSnapshot'),
    'stop-receipt': ('stop_vm', 'StopVm'),
    'restore-prepare-receipt': ('prepare_vm', 'PrepareVm'),
    'restore-prepare-destroy': ('prepare_vm', 'PrepareVm'),
}
RESTORE_STAGES = frozenset(('restore-prepare-receipt', 'restore-prepare-destroy'))


class _AwaitTerminal(Exception):
    """An identified original operation is still pending; never success."""


class CaptureHandoffProbe:
    def __init__(self, runtime, client, physical_running, stage, gate_dir=None):
        if stage not in STAGES or physical_running['session'] != client.session:
            raise ValueError('one selected boundary and original public session required')
        self.runtime, self.client = runtime, client
        self.prior, self.stage = copy.deepcopy(physical_running), stage
        self.path = Path(gate_dir) if gate_dir is not None else None
        self.fd = self.pidfd = None
        self.files, self.accepted = {}, None
        self.terminal_accepted = None
        self.pause_requested = self.candidate_paused = False
        self.host = self.question = self.record = self.action = None
        self.command_sent = self.recovered = False

    def _configuration(self):
        role = self.runtime.services.processes.bindings['host-0']
        if role_facts(Path(role['config']).parent, role['config'],
                      uid=role['uid'], gid=role['gid']) != self.runtime.configs['host-0']:
            raise ValueError('original Host configuration changed')

    def arm(self, deadline, timeout_ms):
        if self.host is not None:
            raise ValueError('one arm per original handoff')
        remaining(deadline)
        self._configuration()
        runtime = self.runtime
        generation = self.prior['context']['binding']['generation']
        if self.stage in RESTORE_STAGES:
            generation += 1  # gate_configuration rejects overflow; no old-generation pidfd adoption.
        native = runtime.services.native_pools[0]
        sandbox = 'sb-' + native['namespace'] + '-' + str(self.prior['local']['sandbox'])
        self.config = gate_configuration(runtime.lease.identity, sandbox, generation, timeout_ms)
        self.deadline = min(deadline, time.monotonic() + timeout_ms / 1000)
        if self.stage not in RESTORE_STAGES:
            self.pidfd = open_original_candidate_pidfd(self.prior['source'])
        if self.stage == 'copy-partial':
            control = private_path(runtime.quota['deployment_control'], directory=True)
            if self.path != control / 'capture-copy-gate':
                raise ValueError('exact private deployment copy gate required')
            self.path.mkdir(mode=0o700)
            self.fd = os.open(self.path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
            self.directory = os.fstat(self.fd)
            gate_publish(self.fd, 'config.json', self.config)
            self._save_file('config.json')
            fault = {'SANDPROOF_TEST_CAPTURE_COPY_GATE': str(self.path)}
        else:
            fault = {'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': STAGES[self.stage][0]}
        runtime.services.processes.stop('host-0')
        self._configuration()
        self.host = runtime.services.processes.start('host-0', env=fault)
        runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        return dict(stage=self.stage, armed=True, public_completion=False)

    def _save_file(self, name):
        value, digest, identity = gate_json(self.path / name,
            16384 if name == 'observation.json' else 1024, directory_fd=self.fd)
        self.files[name] = (digest, identity)
        return value, digest

    def _gate_unchanged(self):
        current = self.path.lstat()
        held = os.fstat(self.fd)
        if (not stat.S_ISDIR(current.st_mode) or current.st_uid != 0
                or stat.S_IMODE(current.st_mode) != 0o700
                or (current.st_dev, current.st_ino) != (self.directory.st_dev, self.directory.st_ino)
                or (held.st_dev, held.st_ino) != (current.st_dev, current.st_ino)
                or set(os.listdir(self.fd)) - {'config.json', 'observation.json', 'ready.json',
                    'ready-stage.json', 'command-stage.json', 'command.json', 'consumed.json'}):
            raise ValueError('original private copy gate changed')
        for name, expected in self.files.items():
            _, digest, identity = gate_json(self.path / name,
                16384 if name == 'observation.json' else 1024, directory_fd=self.fd)
            if (digest, identity) != expected:
                raise ValueError('original sealed copy gate file changed')

    def _read(self, deadline):
        native = self.runtime.services.native_pools[0]
        database = private_path(native['database'])
        identity = (database.stat().st_dev, database.stat().st_ino)
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True,
                                      timeout=min(2, remaining(deadline)))) as conn:
            conn.execute('BEGIN')
            header = conn.execute('SELECT length(state_json),reserve_bytes FROM '
                                  'controller_lifecycle_inbox WHERE id=1').fetchone()
            if header is None or not 0 < header[0] <= header[1] <= 16 * 1024**2:
                raise ValueError('bounded original lifecycle inbox required')
            raw, checksum = conn.execute('SELECT state_json,sha256 FROM '
                                          'controller_lifecycle_inbox WHERE id=1').fetchone()
            if hashlib.sha256(raw).digest() != checksum:
                raise ValueError('original inbox checksum differs')
            inbox = json.loads(raw, object_pairs_hook=unique_keys)
            sessions = [s for s in inbox['sessions'] if s['id'] == self.accepted['session']]
            if len(sessions) != 1 or sessions[0]['current_version'] != self.accepted['accepted_version']:
                raise ValueError('exact current accepted session required')
            entries = [e for e in sessions[0]['entries'] if
                e['intent']['accepted_version'] == self.accepted['accepted_version']
                and e['intent']['request']['binding']['key'] == self.accepted['key']]
            if len(entries) != 1 or entries[0]['local_operation'] is None:
                raise ValueError('one original admitted lifecycle operation required')
            entry = entries[0]
            binding = entry['intent']['request']['binding']
            if (binding['session'] != self.accepted['session']
                    or binding['node'] != self.runtime.services.host_configs[0]['inbox']['node']):
                raise ValueError('accepted intent differs from original session/node')
            local = sessions[0]['local']
            if any(local[k] != self.prior['local'][k] for k in ('sandbox', 'owner')):
                raise ValueError('original public/native mapping changed')
            ident = 'sb-' + native['namespace'] + '-' + str(local['sandbox'])
            operation = 'op-' + native['namespace'] + '-' + str(local['sandbox']) + '-' + str(entry['local_operation'])
            capacity = self.runtime.services.host_configs[0]['native']['maintenance']['capacity']
            count, largest, receipt_size = conn.execute(
                'SELECT count(*),max(length(action_json)),max(length(receipt_json)) FROM outbox '
                'WHERE sandbox_id=? AND operation_id=?', (ident, operation)).fetchone()
            if (not 0 < count <= capacity['max_sql_rows']
                    or largest > capacity['max_sql_value_bytes']
                    or (receipt_size is not None and receipt_size > capacity['max_sql_value_bytes'])):
                raise ValueError('original bounded outbox required before reading')
            rows = conn.execute('SELECT action_json,dispatch_state,receipt_json FROM outbox '
                'WHERE sandbox_id=? AND operation_id=? ORDER BY id', (ident, operation)).fetchall()
            cap = self.runtime.services.host_configs[0]['native']['maintenance']['capacity']['max_sql_value_bytes']
            if any(len(row[0]) > cap or (row[2] is not None and len(row[2]) > cap) for row in rows):
                raise ValueError('original outbox exceeds configured value bound')
            actions = [(json.loads(raw, object_pairs_hook=unique_keys), state, receipt)
                       for raw, state, receipt in rows]
            records = checked_source_records(conn, self.runtime.services.host_configs[0], deadline)
        if (database.stat().st_dev, database.stat().st_ino) != identity:
            raise ValueError('original SQLite replaced during boundary observation')
        return entry, records, actions

    def _original_action(self, actions, entry):
        command = STAGES[self.stage][1]
        matches = [(action, state, receipt) for action, state, receipt in actions
                   if command in action['command']['logical']['command']]
        if len(matches) != 1 or matches[0][1:] != ('dispatched', None):
            raise ValueError('one exact dispatched action with no SQL receipt required')
        action = matches[0][0]
        identity = action['identity']
        generation = self.prior['context']['binding']['generation']
        if self.stage in RESTORE_STAGES:
            generation += 1
        fields = action['command']['logical']['command'][command]
        native = self.runtime.services.native_pools[0]
        expected_operation = ('op-' + native['namespace'] + '-' + str(self.prior['local']['sandbox'])
                              + '-' + str(entry['local_operation']))
        if (identity['protocol_version'] != 1 or identity['operation'] != expected_operation
                or identity['sandbox'] != self.config['sandbox'] or identity['generation'] != generation
                or identity['owner'] != 'tenant-' + str(self.prior['local']['owner'])
                or fields['sandbox'] != self.prior['local']['sandbox']
                or fields['generation'] != generation or identity['epoch'] != fields['epoch']
                or identity['attempt'] != action['command']['attempt']):
            raise ValueError('original command identity differs from accepted operation')
        if self.stage not in RESTORE_STAGES and fields['vm'] != self.prior['context']['binding']['vm']:
            raise ValueError('original physical VM differs from selected command')
        return action

    def _full(self, entry):
        record = self.record
        anchor, requirement = record['work']['anchor'], record['work']['requirement']
        if (any(anchor['local'][k] != self.prior['local'][k] for k in ('sandbox', 'owner'))
                or requirement['resource']['id'] != self.prior['source']['resource_id']
                or requirement['node'] != self.runtime.services.host_configs[0]['inbox']['node']):
            raise ValueError('original source work mapping differs')
        captured = record['capture']
        if captured is None:
            if self.stage != 'freeze-receipt':
                raise ValueError('original admitted Full missing')
            return None
        full = captured['full']; pause = full['pause']
        request = entry['intent']['request']
        if (captured['operation'] != entry['local_operation']
                or pause['binding'] != dict(operation=request['binding'],
                    source_incarnation=request['operation']['source_incarnation'],
                    capture=request['operation']['capture'])
                or pause['source'] != requirement['source'] or pause['source_resource'] != requirement['resource']
                or pause['source_work_binding'] != requirement['work_binding']
                or pause['node'] != requirement['node']):
            raise ValueError('complete original Full differs from accepted source work')
        if self.stage == 'freeze-receipt':
            if any(captured[k] is not None for k in ('action', 'worker', 'completed', 'end')):
                raise ValueError('freeze receipt boundary already has producer results')
        elif captured['action'] is None or captured['worker'] is None:
            raise ValueError('original Full producer identity missing')
        return captured

    def _sealed_materials(self, captured):
        store = private_path(Path(self.runtime.captures[0]['layout_parent']) / 's', directory=True)
        completions = []
        for path in store.iterdir():
            if path.name.startswith('completed-'):
                value = original_json(path, 16384)
                if value['action'] == captured['action']:
                    completions.append(value)
        if len(completions) != 1:
            raise ValueError('one original durable physical completion required')
        # These values come from the real protected completion, not a SQL write
        # or an invented adoption. The existing reader checks all bundle bytes.
        observed = copy.deepcopy(captured)
        observed['completed'] = completions[0]['manifest_digest']
        observed['material_bytes'] = completions[0]['material_bytes']
        return self.runtime._materials(captured['full'], observed)

    def after_acceptance(self, accepted, deadline):
        if self.host is None or self.accepted is not None or accepted['session'] != self.client.session:
            raise ValueError('one original acceptance after arming required')
        self.accepted = copy.deepcopy(accepted)
        deadline = min(deadline, self.deadline)
        if self.stage == 'copy-partial':
            while not (self.path / 'ready.json').exists():
                self._gate_unchanged()
                if self.host.poll() is not None:
                    raise ValueError('original manager exited before copy observation')
                time.sleep(min(.05, remaining(deadline)))
            self._gate_unchanged()
            question, digest = self._save_file('observation.json')
            ready, _ = self._save_file('ready.json')
            validate_gate_ready(ready, digest)
            if (set(question) != {'schema_version', 'run_identity', 'action', 'full_work_binding',
                    'worker', 'source', 'copied_bytes', 'total_bytes'}
                    or question['schema_version'] != 1 or question['run_identity'] != self.config['run_identity']
                    or type(question['copied_bytes']) is not int or type(question['total_bytes']) is not int
                    or not 0 < question['copied_bytes'] < question['total_bytes']):
                raise ValueError('actual incomplete original Memory copy required')
            self.question, self.question_digest = question, digest
        elif self.host.wait(timeout=remaining(deadline)) != 86:
            raise ValueError('original selected manager did not exit86')
        self._configuration()
        entry, records, actions = self._read(deadline)
        expected_kind = 'resume' if self.stage in RESTORE_STAGES else 'unload'
        if entry['intent']['request']['operation']['kind'] != expected_kind:
            raise ValueError('selected boundary has another original lifecycle operation')
        self.action = self._original_action(actions, entry)
        if self.stage in RESTORE_STAGES:
            candidates = [r for r in records if r['restore'] is not None
                          and r['restore']['intent'] == entry['intent']]
            if (len(candidates) != 1 or candidates[0]['restore']['activation'] is not None
                    or candidates[0]['restore']['material_use'] is not True):
                raise ValueError('original LoadPaused pin before activation marker required')
            self.restore_entry = copy.deepcopy(entry)
            self.restore_record = copy.deepcopy(candidates[0])
            resource = self.action['identity']['resource']['id']
            native = self.runtime.services.native_pools[0]
            candidate = original_json(Path(native['chroot_base']) / '.claims' / resource / 'record.json', 16384)
            if candidate['resource_id'] != resource:
                raise ValueError('original loaded candidate record differs')
            claim = Path(native['chroot_base']) / '.claims' / resource
            loaded = original_json(claim / 'restore-loaded.json', 16384)
            intent = original_json(claim / 'restore-intent.json', 16384)
            if (loaded['process'] != {k: candidate[k] for k in ('pid', 'start_time_ticks', 'boot_id')}
                    or loaded['intent_sha256'] != checked_file(claim / 'restore-intent.json', 16384)['sha256']
                    or intent['resource_id'] != resource
                    or intent['manifest_sha256'] != candidates[0]['capture']['completed']):
                raise ValueError('original loaded candidate differs from retained source manifest')
            self.pidfd = open_original_candidate_pidfd(candidate)
            self.candidate = candidate
            if self.stage == 'restore-prepare-destroy':
                self.prepare_files = {name: checked_file(claim / name, 16384) for name in
                    ('record.json', 'restore-intent.json', 'restore-loaded.json')}
                # The manager has actually exited86, so this original audit
                # interrupts no live Host or held observation.
                audit = self.runtime.services.processes.audit('host-0')
                local = local_binding(audit, accepted['session'],
                    expected_node=self.runtime.services.host_configs[0]['inbox']['node'])
                slots = [context for context, phase in audit['host_slots'] if phase == 'Occupied'
                    and context['binding']['sandbox'] == local['sandbox']]
                if (local != self.restore_record['restore']['local'] or len(slots) != 1
                        or slots[0]['binding']['generation'] != self.action['identity']['generation']
                        or slots[0]['binding']['vm'] != self.action['command']['logical']['command']['PrepareVm']['vm']):
                    raise ValueError('exact original physically occupied LoadPaused slot required')
                self.prepare_context = copy.deepcopy(slots[0])

        else:
            candidates = [r for r in records if r['work']['anchor']['request'] == entry['intent']]
            if len(candidates) != 1:
                raise ValueError('one exact original source work required')
            self.record = candidates[0]
            source = {k: self.prior['source'][k] for k in ('pid', 'start_time_ticks', 'boot_id')}
            if self.record['work']['requirement']['source'] != source:
                raise ValueError('original source process differs')
            captured = self._full(entry)
            if self.stage == 'seal-receipt':
                if captured['action'] != self.action or captured['completed'] is not None:
                    raise ValueError('expected physical Full before SQL adoption')
                self.materials = self._sealed_materials(captured)
            elif self.stage == 'stop-receipt':
                if captured['completed'] is None:
                    raise ValueError('Stop before original SQL Full adoption')
                self.materials = self.runtime._materials(captured['full'], captured)
            if self.stage == 'copy-partial':
                self.record = continued_capture_question(self.question, accepted, records, self.prior['source'])
                if self.question['action'] != self.action or self.question['source'] != source:
                    raise ValueError('copy question differs from complete original action/source')
                self.worker_pidfd = open_original_candidate_pidfd(self.question['worker']['process'])
                if question['total_bytes'] > captured['full']['memory_bytes']:
                    raise ValueError('copy observation exceeds original Memory bound')
            if self.stage == 'stop-receipt':
                if not pidfd_exited(self.pidfd):
                    raise ValueError('original Stop did not close the exact source process')
            elif pidfd_exited(self.pidfd) or not original_process(self.prior['source']):
                raise ValueError('original source changed before selected capture boundary')
        return dict(stage=self.stage, original_acceptance_matched=True,
                    action_sha256=hashlib.sha256(json.dumps(self.action, sort_keys=True,
                        separators=(',', ':')).encode()).hexdigest(), public_completion=False)

    def _prepare_candidate_state(self, deadline):
        remaining(deadline)
        if (self.stage != 'restore-prepare-destroy' or self.pidfd is None
                or getattr(self, 'candidate', None) is None):
            raise ValueError('exact terminal-only loaded candidate and original pidfd required')
        if pidfd_exited(self.pidfd) or not original_process(self.candidate):
            raise ValueError('original pre-marker loaded candidate no longer live')
        fields = (Path('/proc') / str(self.candidate['pid']) / 'stat').read_text().rsplit(')', 1)[1].split()
        if (int(fields[19]) != self.candidate['start_time_ticks']
                or pidfd_exited(self.pidfd) or not original_process(self.candidate)):
            raise ValueError('original candidate changed during process-state observation')
        return fields[0]

    def pause_original_prepare(self, deadline):
        """Stop the exact process; T alone is no VM-state or exit observation.

        Prior LoadPaused comes from its original protected loaded record.
        No Firecracker action, guessed PID, new Load or resource release occurs.
        An uncertain pause keeps this scenario's original responsibilities.
        """
        if self.action is None or self.pause_requested or self.recovered:
            raise ValueError('one original pre-marker Prepare observation required')
        deadline = min(deadline, self.deadline)
        self._prepare_candidate_state(deadline)
        self.pause_requested = True
        signal.pidfd_send_signal(self.pidfd, signal.SIGSTOP)
        while self._prepare_candidate_state(deadline) != 'T':
            time.sleep(min(.01, remaining(deadline)))
        self.candidate_paused = True
        return dict(original_pidfd_stopped=True, process_state='T',
                    physical_exit=False, public_completion=False, capacity_released=False)

    def _bind_terminal_acceptance(self, accepted):
        if (self.stage != 'restore-prepare-destroy' or self.accepted is None
                or self.terminal_accepted is not None or not self.candidate_paused):
            raise ValueError('one paused original Prepare and one public Destroy required')
        if (type(accepted) is not dict or set(accepted) != {'session', 'key', 'accepted_version'}
                or accepted != self.client.current_acceptance
                or accepted['session'] != self.accepted['session']
                or type(accepted['accepted_version']) is not int
                or accepted['accepted_version'] != self.accepted['accepted_version'] + 1
                or accepted['key'] == self.accepted['key']
                or any(accepted['key'][k] != self.accepted['key'][k] for k in ('deployment', 'owner'))
                or self.client.destroy_request != dict(window=accepted['key']['window'],
                    sequence=accepted['key']['sequence'], expected_version=self.accepted['accepted_version'])):
            raise ValueError('public Destroy differs from the exact next accepted request')
        self.terminal_accepted = copy.deepcopy(accepted)

    def _terminal_entries(self, inbox):
        if self.stage != 'restore-prepare-destroy' or self.terminal_accepted is None:
            raise ValueError('terminal-specific original acceptance required')
        accepted = self.terminal_accepted
        sessions = [row for row in inbox['sessions'] if row['id'] == self.accepted['session']]
        if len(sessions) != 1:
            raise ValueError('one exact original Host session required')
        session = sessions[0]
        if session['current_version'] == self.accepted['accepted_version'] and session['terminal'] is False:
            raise _AwaitTerminal('public Destroy has not reached the original Host session')
        if session['current_version'] != accepted['accepted_version'] or session['terminal'] is not True:
            raise ValueError('another current Host version or nonterminal state')
        original = [entry for entry in session['entries'] if entry['intent'] == self.restore_entry['intent']]
        terminal = [entry for entry in session['entries'] if
            entry['intent']['accepted_version'] == accepted['accepted_version']
            and entry['intent']['request']['binding']['key'] == accepted['key']]
        if (len(original) != 1 or original[0] != self.restore_entry or len(terminal) != 1
                or any(session['local'][k] != self.prior['local'][k] for k in ('sandbox', 'owner'))):
            raise ValueError('original Resume, public Destroy or native mapping changed')
        entry = terminal[0]; intent = entry['intent']; binding = intent['request']['binding']
        prior = self.restore_entry['intent']
        if (intent['request']['operation'] != {'kind': 'destroy'}
                or binding['session'] != self.accepted['session']
                or binding['node'] != prior['request']['binding']['node']
                or binding['protocol_version'] != prior['request']['binding']['protocol_version']
                or binding['expected'] != {'state': 'version', 'version': self.accepted['accepted_version']}
                or intent['center'] != prior['center'] or intent['retention'] != prior['retention']):
            raise ValueError('complete original Destroy intent differs from the accepted successor')
        if entry['local_operation'] is None:
            raise _AwaitTerminal('Host received Destroy but native Kill is not yet admitted')
        return original[0], entry, session['local']

    def _terminal_outbox(self, conn, local, operation, capacity):
        native = self.runtime.services.native_pools[0]
        sandbox = 'sb-' + native['namespace'] + '-' + str(local['sandbox'])
        op = 'op-' + native['namespace'] + '-' + str(local['sandbox']) + '-' + str(operation)
        count, largest, receipt_size = conn.execute(
            'SELECT count(*),max(length(action_json)),max(length(receipt_json)) FROM outbox '
            'WHERE sandbox_id=? AND operation_id=?', (sandbox, op)).fetchone()
        if (not 0 < count <= capacity['max_sql_rows'] or largest > capacity['max_sql_value_bytes']
                or (receipt_size is not None and receipt_size > capacity['max_sql_value_bytes'])):
            raise ValueError('original bounded actions missing or retired before terminal evidence')
        result = []
        for raw, state, receipt in conn.execute('SELECT action_json,dispatch_state,receipt_json '
                'FROM outbox WHERE sandbox_id=? AND operation_id=? ORDER BY id', (sandbox, op)):
            action = json.loads(raw, object_pairs_hook=unique_keys)
            identity = action['identity']; command = action['command']
            if (identity['sandbox'] != sandbox or identity['operation'] != op
                    or identity['owner'] != self.action['identity']['owner']
                    or identity['protocol_version'] != self.action['identity']['protocol_version']
                    or identity['generation'] != self.action['identity']['generation']
                    or identity['attempt'] != command['attempt']):
                raise ValueError('terminal observation action has a foreign complete identity')
            result.append((action, state, None if receipt is None else
                           json.loads(receipt, object_pairs_hook=unique_keys)))
        return result

    def _read_terminal(self, deadline):
        """Separate terminal-only SQL snapshot; the five old stages stay strict."""
        native = self.runtime.services.native_pools[0]
        database = private_path(native['database'])
        identity = (database.stat().st_dev, database.stat().st_ino)
        capacity = self.runtime.services.host_configs[0]['native']['maintenance']['capacity']
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True,
                                      timeout=min(2, remaining(deadline)))) as conn:
            conn.execute('BEGIN')
            header = conn.execute('SELECT length(state_json),reserve_bytes FROM '
                                  'controller_lifecycle_inbox WHERE id=1').fetchone()
            if header is None or not 0 < header[0] <= header[1] <= min(capacity['max_sql_value_bytes'], 16 * 1024**2):
                raise ValueError('bounded original Host inbox required')
            raw, checksum = conn.execute('SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1').fetchone()
            if hashlib.sha256(raw).digest() != checksum:
                raise ValueError('original terminal inbox checksum differs')
            original, terminal, local = self._terminal_entries(json.loads(raw, object_pairs_hook=unique_keys))
            sandbox = 'sb-' + native['namespace'] + '-' + str(local['sandbox'])
            size = conn.execute('SELECT length(state_blob) FROM sandboxes WHERE sandbox_id=?', (sandbox,)).fetchone()
            if size is None or not 0 < size[0] <= capacity['max_sql_value_bytes']:
                raise ValueError('original native terminal row missing or retired before evidence')
            raw, owner = conn.execute('SELECT state_blob,owner_id FROM sandboxes WHERE sandbox_id=?', (sandbox,)).fetchone()
            state = json.loads(raw, object_pairs_hook=unique_keys)
            if owner != self.action['identity']['owner'] or state['owner'] != local['owner']:
                raise ValueError('original native terminal owner changed')
            sources = checked_source_records(conn, self.runtime.services.host_configs[0], deadline)
            resume_actions = self._terminal_outbox(conn, local, original['local_operation'], capacity)
            terminal_actions = self._terminal_outbox(conn, local, terminal['local_operation'], capacity)
        if (database.stat().st_dev, database.stat().st_ino) != identity:
            raise ValueError('original SQLite replaced during terminal snapshot')
        return dict(original=original, terminal=terminal, local=local, native=state,
                    sources=sources, resume_actions=resume_actions, terminal_actions=terminal_actions)

    def _terminal_inventory(self, snapshot):
        rows = snapshot['resume_actions'] + snapshot['terminal_actions']
        prepared = [row for row in rows if 'PrepareVm' in row[0]['command']['logical']['command']]
        if len(prepared) != 1 or prepared[0][0] != self.action:
            raise ValueError('a missing, replaced or second Prepare action appeared')
        if any('ActivateVm' in row[0]['command']['logical']['command'] for row in rows):
            raise ValueError('pre-marker Destroy must never create an Activate action')
        return prepared[0]

    @staticmethod
    def _success_observation(row, result_kind):
        action, state, receipt = row
        if state in ('ready', 'dispatched') and receipt is None:
            return None
        if (state != 'receipted' or type(receipt) is not dict
                or receipt['identity'] != action['identity']):
            raise ValueError('complete original action receipt required')
        outcome = receipt['outcome']
        if outcome['status'] == 'unknown':
            return None  # Actual tagged uncertainty remains pending, never success.
        if outcome['status'] != 'observed_success':
            raise ValueError('original action did not have an observed successful result')
        observation = outcome['details']
        if (observation['resource'] != action['identity']['resource']
                or observation['command'] != action['command']['logical']['command']
                or set(observation['result']) != {result_kind}):
            raise ValueError('original command-specific receipt resource or kind differs')
        return observation['result'][result_kind]

    def _terminal_pause_barrier(self, snapshot):
        prepared = self._terminal_inventory(snapshot)
        if prepared[1:] != ('dispatched', None):
            raise ValueError('paused original Prepare gained a receipt before terminal qualification')
        record = [row for row in snapshot['sources'] if row['restore'] is not None
                  and row['restore']['intent'] == self.restore_entry['intent']]
        if (len(record) != 1 or record[0]['work'] != self.restore_record['work']
                or record[0]['capture'] != self.restore_record['capture']
                or record[0]['restore'] != self.restore_record['restore']
                or record[0]['retirement'] is not None
                or record[0]['restore']['activation'] is not None
                or record[0]['restore']['material_use'] is not True
                or material_source_charge(record[0]) != material_source_charge(self.restore_record)
                or material_source_charge(record[0]) <= 0):
            raise ValueError('unknown Prepare lost original source, pin or positive material responsibility')
        native = snapshot['native']; operation = native['current_operation']
        if (native['terminal'] is not True or native['phase'] != 'Revoking'
                or native['generation'] != self.action['identity']['generation']
                or native['current_vm'] is not None or native['net_slot'] is not None
                or operation is None or operation['kind'] != 'Kill'
                or operation['id'] != snapshot['terminal']['local_operation']
                or operation['epoch'] != native['epoch']):
            raise ValueError('original native Kill and loaded candidate are not current')
        fences = [row for row in snapshot['terminal_actions'] if 'InstallFence' in row[0]['command']['logical']['command']
                  and row[0]['command']['logical']['command']['InstallFence']['terminal'] is True]
        if not fences:
            raise _AwaitTerminal('original terminal Fence not yet scheduled')
        if len(fences) != 1:
            raise ValueError('more than one terminal Fence requires separate evidence')
        action = fences[0][0]
        if (action['command']['logical']['command']['InstallFence'] != dict(
                sandbox=snapshot['local']['sandbox'], epoch=native['epoch'], terminal=True)
                or action['identity']['epoch'] != native['epoch']
                or native['epoch'] < self.action['identity']['epoch']):
            raise ValueError('original terminal Fence action identity differs')
        observed = self._success_observation(fences[0], 'Fence')
        if observed is None:
            raise _AwaitTerminal('original terminal Fence has no successful SQL receipt yet')
        if observed != dict(epoch=native['epoch'], terminal=True):
            raise ValueError('original terminal Fence observation differs')
        native_pool = self.runtime.services.native_pools[0]
        path = Path(native_pool['node_state_dir']) / 'fences' / self.config['sandbox']
        fence = original_json(path.with_suffix('.json'), 16384)
        if (set(fence) != {'epoch', 'terminal'} or type(fence['epoch']) is not int
                or type(fence['terminal']) is not bool or fence != observed):
            raise ValueError('actual original terminal fence file differs from SQL receipt')
        return fence

    def recover_after_destroy(self, accepted, deadline, *, observe_peer):
        """Continue the same stopped process only after actual terminal fencing."""
        self._bind_terminal_acceptance(accepted)
        deadline = min(deadline, self.deadline)
        if self._prepare_candidate_state(deadline) != 'T' or self.host.poll() != 86:
            raise ValueError('original exited manager and stopped loaded candidate required')
        self._configuration()
        process = self.runtime.services.processes.start('host-0',
            env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'cleanup_vm'})
        self.terminal_host = process
        if process is self.host or process.pid == self.host.pid:
            raise ValueError('fresh original Host required to receive public Destroy')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        while True:
            if self._prepare_candidate_state(deadline) != 'T':
                raise ValueError('candidate was externally continued before exact terminal barrier')
            try:
                snapshot = self._read_terminal(deadline)
                fence = self._terminal_pause_barrier(snapshot)
                break
            except _AwaitTerminal:
                time.sleep(min(.05, remaining(deadline)))
        self._configuration()
        native = self.runtime.services.native_pools[0]
        claim = Path(native['chroot_base']) / '.claims' / self.candidate['resource_id']
        if any(checked_file(claim / name, 16384) != value for name, value in self.prepare_files.items()):
            raise ValueError('original loaded process files changed before terminal continuation')
        if self._prepare_candidate_state(deadline) != 'T':
            raise ValueError('original loaded process is no longer held stopped')
        if not callable(observe_peer):
            raise ValueError('one original peer observation required before continuation')
        peer = observe_peer()
        if self._prepare_candidate_state(deadline) != 'T':
            raise ValueError('original candidate changed during peer progress observation')
        self.terminal_barrier = copy.deepcopy(snapshot)
        signal.pidfd_send_signal(self.pidfd, signal.SIGCONT)
        self.continue_sent = self.recovered = True
        return dict(public_destroy_reached_host=True, terminal_fence_observed=fence,
                    unknown_prepare_still_dispatched=True, material_pin_and_charge_retained=True,
                    peer_observation=peer, original_pidfd_continue_sent=True,
                    public_completion=False, capacity_released=False)

    def assert_terminal_before_gc(self, deadline):
        """Read once at the original CleanupVm-success/pre-SQL-receipt cut.

        Unacknowledged CleanupVm prevents logical reclamation and Retire. The
        actual exit86 freezes this Host's original Prepared/Stop rows before
        any deletion. No polling race, missing-row inference or new Rust gate.
        """
        if self.stage != 'restore-prepare-destroy' or not self.recovered or not self.continue_sent:
            raise ValueError('original terminal barrier and explicit pidfd continuation required')
        deadline = min(deadline, self.deadline)
        if (self.terminal_host is not self.runtime.services.processes.children['host-0']
                or self.terminal_host.wait(timeout=remaining(deadline)) != 86):
            raise ValueError('exact original cleanup receipt-loss manager must exit86')
        self._configuration()
        snapshot = self._read_terminal(deadline)
        prepared = self._terminal_inventory(snapshot)
        prepared_result = self._success_observation(prepared, 'Prepared')
        if prepared_result != dict(pid=self.candidate['pid'], start_ticks=self.candidate['start_time_ticks'],
                                   boot_id=self.candidate['boot_id']):
            raise ValueError('original Prepared process differs at fixed cleanup boundary')
        stops = [row for row in snapshot['terminal_actions'] if 'StopVm' in row[0]['command']['logical']['command']]
        cleanups = [row for row in snapshot['terminal_actions'] if 'CleanupVm' in row[0]['command']['logical']['command']]
        if len(stops) != 1 or len(cleanups) != 1:
            raise ValueError('one original Stop and one unacknowledged Cleanup boundary required')
        vm = self.action['command']['logical']['command']['PrepareVm']['vm']
        expected = dict(sandbox=snapshot['local']['sandbox'], generation=self.action['identity']['generation'], vm=vm)
        stop = stops[0][0]; cleanup = cleanups[0][0]
        epoch = self.terminal_barrier['native']['epoch']
        if (stop['command']['logical']['command']['StopVm'] != dict(expected, epoch=epoch)
                or stop['identity']['epoch'] != epoch
                or stop['identity']['resource'] != self.action['identity']['resource']
                or cleanup['command']['logical']['command']['CleanupVm'] != expected
                or cleanup['identity']['epoch'] != epoch
                or cleanup['identity']['resource'] != self.action['identity']['resource']
                or cleanups[0][1:] != ('dispatched', None)):
            raise ValueError('fixed cleanup cut does not name the exact original terminal VM')
        stopped = self._success_observation(stops[0], 'Stopped')
        if stopped != dict(pid=self.candidate['pid'], start_ticks=self.candidate['start_time_ticks']):
            raise ValueError('original Stop process differs at fixed cleanup boundary')
        if not pidfd_exited(self.pidfd) or original_process(self.candidate):
            raise ValueError('exact loaded candidate has not actually exited')
        # The cut's pending Cleanup SQL receipt forbids the original Retire path.
        # After stable evidence, reuse ordinary recovery rather than inventing ack.
        self.terminal_cut = copy.deepcopy(snapshot)
        self._configuration()
        process = self.runtime.services.processes.start('host-0')
        if process is self.terminal_host or process.pid == self.terminal_host.pid:
            raise ValueError('fresh original manager required after cleanup receipt-loss cut')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        self._configuration()
        return dict(original_prepare_observed=True, original_stop_observed=True,
                    original_candidate_exited=True, no_second_prepare_or_activate=True,
                    cleanup_success_pre_sql_exit86=True, cleanup_sql_ack_pending_at_cut=True,
                    original_recovery_resumed=True, public_completion=False, capacity_released=False)

    def finish(self, deadline):
        if self.stage == 'restore-prepare-destroy':
            raise ValueError('terminal handoff requires recover_after_destroy with the exact Destroy acceptance')
        if self.action is None or self.recovered:
            raise ValueError('one observed original handoff required')
        deadline = min(deadline, self.deadline)
        if self.stage == 'copy-partial':
            self._gate_unchanged()
            command = dict(schema_version=1, run_identity=self.config['run_identity'],
                observation_sha256=self.question_digest,
                full_work_binding=self.question['full_work_binding'], command='fail')
            gate_publish(self.fd, 'command.json', command)
            self._save_file('command.json')
            self.command_sent = True
            while True:
                self._gate_unchanged()
                _, records, _ = self._read(deadline)
                record = continued_capture_question(self.question, self.accepted, records, self.prior['source'])
                end = record['capture']['end']
                if end is not None:
                    observation = end.get('observation')
                    if (end['state'] != 'worker_ended' or observation is None
                            or observation['action'] != self.question['action']
                            or observation['worker'] != self.question['worker']
                            or observation['full_work_binding'] != self.question['full_work_binding']
                            or observation['source'] != self.question['source']
                            or observation['outcome']['kind'] != 'returned'
                            or not 200 <= observation['outcome']['http_status'] <= 599
                            or record['capture']['completed'] is not None):
                        raise ValueError('partial copy must have its actual Returned End, never NoRequest')
                    if pidfd_exited(self.worker_pidfd) and not original_process(self.question['worker']['process']):
                        break
                time.sleep(min(.05, remaining(deadline)))
            consumed, _ = self._save_file('consumed.json')
            if consumed != command:
                raise ValueError('original copy failure command not consumed exactly')
            self._gate_unchanged()
            self.recovered = True
            return dict(stage=self.stage, actual_worker_end=True, no_request_inferred=False,
                        public_completion=False, capacity_released=False)
        self._configuration()
        process = self.runtime.services.processes.start('host-0')
        if process is self.host or process.pid == self.host.pid:
            raise ValueError('fresh original manager required after receipt loss')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        self._configuration()
        self.recovered = True
        return dict(stage=self.stage, exit_code=86, fault_environment_removed=True,
                    same_product_configuration=True, public_completion=False)

    def assert_restored_candidate(self, physical_running):
        if self.stage != 'restore-prepare-receipt' or not self.recovered:
            raise ValueError('original recovered LoadPaused observation required')
        if (physical_running['session'] != self.accepted['session']
                or physical_running['source'] != self.candidate
                or physical_running['context']['binding']['generation'] != self.action['identity']['generation']
                or physical_running['context']['binding']['vm'] !=
                    self.action['command']['logical']['command']['PrepareVm']['vm']):
            raise ValueError('recovery replaced the original pre-marker loaded candidate')
        return dict(same_loaded_candidate=True, new_load_inferred=False)

    def cleanup(self):
        """Only after original product GC: retire our exact consumed gate files."""
        if self.stage != 'copy-partial':
            return None
        expected = {'config.json', 'observation.json', 'ready.json', 'command.json', 'consumed.json'}
        if (not self.runtime.gc_complete or not self.runtime.physical_complete
                or not self.recovered or set(self.files) != expected):
            raise ValueError('actual End, exact gate files and original product GC required')
        self._gate_unchanged()
        if set(os.listdir(self.fd)) != expected:
            raise ValueError('unknown gate entries retain original responsibility')
        parent = os.open(self.path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            for name in ('consumed.json', 'command.json', 'ready.json', 'observation.json', 'config.json'):
                _, digest, identity = gate_json(self.path / name,
                    16384 if name == 'observation.json' else 1024, directory_fd=self.fd)
                if (digest, identity) != self.files[name]:
                    raise ValueError('original gate file changed before exact retirement')
                descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
                                     dir_fd=self.fd)
                try:
                    meta = os.fstat(descriptor)
                    if (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns,
                            meta.st_ctime_ns, meta.st_mode, meta.st_uid, meta.st_nlink) != identity:
                        raise ValueError('original gate FD changed before retirement')
                    os.unlink(name, dir_fd=self.fd)
                    if os.fstat(descriptor).st_nlink != 0:
                        raise ValueError('original gate file not unlinked')
                finally:
                    os.close(descriptor)
            os.fsync(self.fd)
            current = self.path.lstat()
            if (current.st_dev, current.st_ino) != (self.directory.st_dev, self.directory.st_ino):
                raise ValueError('original gate directory replaced before removal')
            os.rmdir(self.path.name, dir_fd=parent)
            os.fsync(parent)
        finally:
            os.close(parent)
        return dict(exact_consumed_gate_removed=True)

    def terminal_failure_state(self):
        """Preserve exact stopped-process identity; never signal or infer GC."""
        if self.stage != 'restore-prepare-destroy':
            return None
        result = dict(candidate=copy.deepcopy(getattr(self, 'candidate', None)),
                      pause_requested=self.pause_requested,
                      continue_sent=getattr(self, 'continue_sent', False),
                      public_completion=False, capacity_released=False,
                      action='none; preserve original process and resource responsibility')
        if self.pidfd is not None and result['candidate'] is not None:
            try:
                result['pidfd_exited'] = pidfd_exited(self.pidfd)
                if not result['pidfd_exited'] and original_process(self.candidate):
                    fields = (Path('/proc') / str(self.candidate['pid']) / 'stat').read_text().rsplit(')', 1)[1].split()
                    if int(fields[19]) != self.candidate['start_time_ticks']:
                        raise ValueError('candidate identity changed while preserving failure evidence')
                    result['process_state'] = fields[0]
            except (OSError, ValueError):
                result['process_observation'] = 'unknown; do not adopt a replacement or release resources'
        return result

    def close(self):
        """Close observation descriptors only; uncertain effects and files remain."""
        for name in ('fd', 'pidfd', 'worker_pidfd'):
            descriptor = getattr(self, name, None)
            if descriptor is not None:
                os.close(descriptor)
                setattr(self, name, None)


def self_test():
    """Original input checks and terminal handoff checks; no Host or VM.

    Two tests own a small Linux child to check real pidfd stop/continue order.
    """
    import unittest
    import subprocess
    import select
    import sys
    from contextlib import contextmanager
    from unittest.mock import Mock, patch
    from types import SimpleNamespace as NS
    class IdentityTests(unittest.TestCase):
        def fixture(self,stage='freeze-receipt'):
            p=object.__new__(CaptureHandoffProbe)
            p.stage=stage;p.prior=dict(local=dict(sandbox=4,owner=7),context=dict(binding=dict(generation=2,vm=8)))
            p.config=dict(sandbox='sb-0123456789abcdef-4')
            p.runtime=NS(services=NS(native_pools=[dict(namespace='0123456789abcdef')]))
            command=STAGES[stage][1];generation=3 if stage in RESTORE_STAGES else 2
            fields=dict(sandbox=4,epoch=9,generation=generation,vm=10 if generation==3 else 8)
            action=dict(identity=dict(protocol_version=1,operation='op-0123456789abcdef-4-3',sandbox=p.config['sandbox'],
                generation=generation,owner='tenant-7',epoch=9,attempt=0),command=dict(logical=dict(command={command:fields}),attempt=0))
            return p,action,dict(local_operation=3)
        def test_exact_action_and_wrong_identity_are_distinguished(self):
            p,action,entry=self.fixture()
            self.assertEqual(p._original_action([(action,'dispatched',None)],entry),action)
            for field,value in [('protocol_version',2),('operation','op-0123456789abcdef-4-4'),('generation',3),
                    ('owner','tenant-8'),('sandbox','sb-0123456789abcdef-5'),('epoch',10),('attempt',1)]:
                wrong=copy.deepcopy(action);wrong['identity'][field]=value
                with self.subTest(field=field),self.assertRaises(ValueError):p._original_action([(wrong,'dispatched',None)],entry)
            wrong=copy.deepcopy(action);wrong['command']['logical']['command']['FreezeVm']['vm']=9
            with self.assertRaises(ValueError):p._original_action([(wrong,'dispatched',None)],entry)
        def test_missing_duplicate_or_already_receipted_is_not_selected_boundary(self):
            p,action,entry=self.fixture()
            for rows in ([],[(action,'ready',None)],[(action,'receipted',b'{}')],[(action,'dispatched',None)]*2):
                with self.subTest(rows=len(rows)),self.assertRaises(ValueError):p._original_action(rows,entry)
        def test_restore_uses_next_generation_and_same_candidate(self):
            p,action,entry=self.fixture('restore-prepare-receipt')
            self.assertEqual(p._original_action([(action,'dispatched',None)],entry),action)
            wrong=copy.deepcopy(action);wrong['identity']['generation']=2
            with self.assertRaises(ValueError):p._original_action([(wrong,'dispatched',None)],entry)
            p.recovered=True;p.accepted=dict(session='global-session');p.action=action
            p.candidate=dict(resource_id='vm-0123456789abcdef-10',pid=123,start_time_ticks=456,boot_id='boot')
            physical=dict(session='global-session',source=copy.deepcopy(p.candidate),context=dict(binding=dict(generation=3,vm=10)))
            self.assertTrue(p.assert_restored_candidate(physical)['same_loaded_candidate'])
            for field,value in [('pid',124),('start_time_ticks',457),('resource_id','vm-0123456789abcdef-11')]:
                wrong=copy.deepcopy(physical);wrong['source'][field]=value
                with self.subTest(field=field),self.assertRaises(ValueError):p.assert_restored_candidate(wrong)
        def test_partial_gate_configuration_never_adopts_generation_overflow(self):
            from lifecycle_unload_runtime import gate_configuration
            self.assertEqual(gate_configuration('a'*64,'sb-0123456789abcdef-4',3,1000)['generation'],3)
            for generation in (0,2**64):
                with self.assertRaises(ValueError):gate_configuration('a'*64,'sb-0123456789abcdef-4',generation,1000)
        def terminal_fixture(self, *, bind=True):
            p, action, _ = self.fixture('restore-prepare-destroy')
            p.action = action
            action['identity']['resource'] = dict(kind='vm', id='vm-0123456789abcdef-10')
            p.accepted = dict(session='session', accepted_version=3,
                key=dict(deployment='deployment', owner='7', window=1, sequence=3))
            terminal = dict(session='session', accepted_version=4,
                key=dict(deployment='deployment', owner='7', window=1, sequence=4))
            node = dict(deployment_id='deployment', node='node')
            original = dict(accepted_version=3, center={'center':'original'}, retention={'window':1},
                request=dict(binding=dict(protocol_version=2, node=node, session='session',
                    key=p.accepted['key'], expected=dict(state='version', version=2)),
                    operation=dict(kind='resume')))
            ending = copy.deepcopy(original); ending['accepted_version'] = 4
            ending['request']['binding']['key'] = terminal['key']
            ending['request']['binding']['expected']['version'] = 3
            ending['request']['operation'] = {'kind':'destroy'}
            p.restore_entry = dict(intent=original, local_operation=3)
            ending_entry = dict(intent=ending, local_operation=4)
            inbox = dict(sessions=[dict(id='session', current_version=4, terminal=True,
                local=dict(sandbox=4, owner=7, operation=1), entries=[copy.deepcopy(p.restore_entry), ending_entry])])
            p.client = NS(current_acceptance=terminal,
                destroy_request=dict(window=1, sequence=4, expected_version=3))
            p.terminal_accepted = None; p.candidate_paused = True
            p.pause_requested = False; p.recovered = False
            if bind:
                p._bind_terminal_acceptance(terminal)
            return p, terminal, inbox

        @staticmethod
        def receipt(action, kind, value):
            return dict(identity=copy.deepcopy(action['identity']), outcome=dict(status='observed_success',
                details=dict(resource=copy.deepcopy(action['identity']['resource']),
                    command=copy.deepcopy(action['command']['logical']['command']), result={kind:value})))

        def test_terminal_entries_keep_both_complete_acceptances_and_local_mapping(self):
            p, terminal, inbox = self.terminal_fixture()
            original, ending, local = p._terminal_entries(inbox)
            self.assertEqual(original, p.restore_entry)
            self.assertEqual(ending['intent']['request']['binding']['key'], terminal['key'])
            self.assertEqual(local['owner'], 7)
            pending = copy.deepcopy(inbox)
            pending['sessions'][0].update(current_version=3, terminal=False)
            with self.assertRaises(_AwaitTerminal):
                p._terminal_entries(pending)
            for mutate in (
                    lambda row: row.update(current_version=5),
                    lambda row: row['local'].update(owner=8),
                    lambda row: row['entries'][0]['intent'].update(accepted_version=2),
                    lambda row: row['entries'][1]['intent']['request']['binding']['expected'].update(version=2),
                    lambda row: row['entries'][1]['intent']['request'].update(operation={'kind':'renew'})):
                wrong = copy.deepcopy(inbox); mutate(wrong['sessions'][0])
                with self.assertRaises(ValueError):
                    p._terminal_entries(wrong)

        def test_terminal_public_acceptance_is_not_an_unrelated_or_repeated_request(self):
            for mutate in (
                    lambda a: a.update(session='another'),
                    lambda a: a.update(accepted_version=5),
                    lambda a: a['key'].update(owner='8'),
                    lambda a: a['key'].update(deployment='another'),
                    lambda a: a['key'].update(sequence=3)):
                p, terminal, _ = self.terminal_fixture(bind=False)
                wrong = copy.deepcopy(terminal); mutate(wrong)
                p.client.current_acceptance = wrong
                with self.assertRaises(ValueError):
                    p._bind_terminal_acceptance(wrong)
                self.assertIsNone(p.terminal_accepted)
            p, terminal, _ = self.terminal_fixture()
            with self.assertRaises(ValueError):
                p._bind_terminal_acceptance(terminal)

        def test_terminal_inventory_and_complete_receipts_reject_replacement_or_activation(self):
            p, _, _ = self.terminal_fixture()
            snapshot = dict(resume_actions=[(p.action, 'dispatched', None)], terminal_actions=[])
            self.assertEqual(p._terminal_inventory(snapshot)[0], p.action)
            duplicate = copy.deepcopy(snapshot); duplicate['terminal_actions'] = duplicate['resume_actions']
            with self.assertRaises(ValueError):
                p._terminal_inventory(duplicate)
            activated = copy.deepcopy(snapshot); action = copy.deepcopy(p.action)
            action['command']['logical']['command'] = {'ActivateVm': {'sandbox':4}}
            activated['terminal_actions'].append((action, 'ready', None))
            with self.assertRaises(ValueError):
                p._terminal_inventory(activated)
            value = dict(pid=123, start_ticks=456, boot_id='boot')
            receipt = self.receipt(p.action, 'Prepared', value)
            self.assertEqual(p._success_observation((p.action, 'receipted', receipt), 'Prepared'), value)
            for mutate in (
                    lambda r: r['identity'].update(epoch=10),
                    lambda r: r['outcome']['details']['resource'].update(id='another'),
                    lambda r: r['outcome']['details']['command']['PrepareVm'].update(vm=11)):
                wrong = copy.deepcopy(receipt); mutate(wrong)
                with self.assertRaises(ValueError):
                    p._success_observation((p.action, 'receipted', wrong), 'Prepared')
            unknown = dict(identity=p.action['identity'], outcome=dict(status='unknown', details={}))
            self.assertIsNone(p._success_observation((p.action, 'receipted', unknown), 'Prepared'))

        def test_terminal_barrier_requires_actual_fence_pending_prepare_and_retained_pin(self):
            p, _, inbox = self.terminal_fixture()
            capture = dict(full=dict(state_bytes=64, memory_bytes=128, disk_bytes=256),
                           reserved_material_bytes=2*(64+128+256)+2*262144+65536)
            p.restore_record = dict(work={}, capture=capture, retirement=None,
                restore=dict(intent=p.restore_entry['intent'], activation=None, material_use=True))
            fence = copy.deepcopy(p.action)
            fence['identity'].update(epoch=10, operation='op-0123456789abcdef-4-4',
                                     resource=dict(kind='sandbox', id=p.config['sandbox']))
            fence['command']['logical']['command'] = dict(InstallFence=dict(sandbox=4, epoch=10, terminal=True))
            receipt = self.receipt(fence, 'Fence', dict(epoch=10, terminal=True))
            snapshot = dict(original=p.restore_entry, terminal=inbox['sessions'][0]['entries'][1],
                local=inbox['sessions'][0]['local'], sources=[copy.deepcopy(p.restore_record)],
                native=dict(terminal=True, phase='Revoking', generation=3, current_vm=None, net_slot=None, epoch=10,
                    current_operation=dict(id=4, kind='Kill', epoch=10)),
                resume_actions=[(p.action, 'dispatched', None)], terminal_actions=[(fence, 'receipted', receipt)])
            p.runtime.services.native_pools[0]['node_state_dir'] = '/unused-unit-fixture'
            with patch.dict(globals(), original_json=lambda *_a, **_k: {'epoch':10, 'terminal':True}):
                self.assertEqual(p._terminal_pause_barrier(snapshot), {'epoch':10, 'terminal':True})
                wrong = copy.deepcopy(snapshot); wrong['sources'][0]['restore']['material_use'] = False
                with self.assertRaises(ValueError):p._terminal_pause_barrier(wrong)
                wrong = copy.deepcopy(snapshot); wrong['sources'][0]['restore']['activation'] = p.action
                with self.assertRaises(ValueError):p._terminal_pause_barrier(wrong)
                wrong = copy.deepcopy(snapshot)
                wrong['resume_actions'][0] = (p.action, 'receipted', self.receipt(p.action, 'Prepared', {}))
                with self.assertRaises(ValueError):p._terminal_pause_barrier(wrong)
            with patch.dict(globals(), original_json=lambda *_a, **_k: {'epoch':10, 'terminal':False}):
                with self.assertRaises(ValueError):p._terminal_pause_barrier(snapshot)

        @contextmanager
        def owned_candidate(self):
            child = subprocess.Popen([sys.executable, '-B', '-c',
                "import os,sys; os.write(1,b'ready\\n'); sys.stdin.buffer.read(1)"],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            descriptor = None
            try:
                self.assertTrue(select.select([child.stdout], [], [], 2)[0])
                self.assertEqual(child.stdout.readline(), b'ready\n')
                fields = (Path('/proc') / str(child.pid) / 'stat').read_text().rsplit(')', 1)[1].split()
                candidate = dict(resource_id='unit-owned-child', pid=child.pid,
                    start_time_ticks=int(fields[19]), boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
                descriptor = open_original_candidate_pidfd(candidate)
                p, terminal, _ = self.terminal_fixture(bind=False)
                p.candidate = candidate; p.pidfd = descriptor
                p.candidate_paused = False; p.deadline = time.monotonic()+5
                p.host = NS(pid=10101, poll=lambda:86)
                p.prepare_files = {}; p._configuration = Mock()
                p.runtime.services.native_pools[0]['chroot_base'] = '/unused-unit-fixture'
                p.runtime.services.processes = NS(start=Mock(return_value=NS(pid=10102)))
                p.runtime.services.wait_ready = Mock()
                yield p, terminal
            finally:
                if descriptor is not None and child.poll() is None:
                    signal.pidfd_send_signal(descriptor, signal.SIGCONT)
                    signal.pidfd_send_signal(descriptor, signal.SIGKILL)
                child.stdin.close()
                child.wait(timeout=2)
                child.stdout.close(); child.stderr.close()
                if descriptor is not None:os.close(descriptor)

        def test_owned_pidfd_continues_only_after_terminal_barrier_and_peer_query(self):
            with self.owned_candidate() as (p, terminal):
                p.pause_original_prepare(p.deadline)
                self.assertEqual(p._prepare_candidate_state(p.deadline), 'T')
                order = []
                p._read_terminal = Mock(return_value={})
                def barrier(_snapshot):
                    self.assertEqual(p._prepare_candidate_state(p.deadline), 'T')
                    order.append('terminal-fence'); return {'epoch':10, 'terminal':True}
                p._terminal_pause_barrier = barrier
                def peer():
                    self.assertEqual(p._prepare_candidate_state(p.deadline), 'T')
                    order.append('peer-query'); return {'unit_peer_returned':True}
                result = p.recover_after_destroy(terminal, p.deadline, observe_peer=peer)
                self.assertEqual(order, ['terminal-fence', 'peer-query'])
                p.runtime.services.processes.start.assert_called_once_with('host-0',
                    env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'cleanup_vm'})
                self.assertTrue(result['original_pidfd_continue_sent'])
                self.assertFalse(result['public_completion'])
                while p._prepare_candidate_state(p.deadline) == 'T':
                    time.sleep(min(.001, remaining(p.deadline)))

        def test_rejected_terminal_barrier_keeps_exact_owned_process_stopped(self):
            with self.owned_candidate() as (p, terminal):
                p.pause_original_prepare(p.deadline)
                p._read_terminal = Mock(return_value={})
                p._terminal_pause_barrier = Mock(side_effect=ValueError('wrong original fence'))
                peer = Mock()
                with self.assertRaises(ValueError):
                    p.recover_after_destroy(terminal, p.deadline, observe_peer=peer)
                peer.assert_not_called()
                self.assertFalse(getattr(p, 'continue_sent', False))
                failure = p.terminal_failure_state()
                self.assertEqual(failure['candidate'], p.candidate)
                self.assertEqual(failure['process_state'], 'T')
                self.assertFalse(failure['pidfd_exited'])
                self.assertFalse(failure['capacity_released'])

        def cleanup_cut_fixture(self):
            p, _, _ = self.terminal_fixture()
            p.recovered = p.continue_sent = True
            p.deadline = time.monotonic()+5
            p.candidate = dict(resource_id='vm-0123456789abcdef-10', pid=123,
                               start_time_ticks=456, boot_id='boot')
            p.pidfd = 99999
            p.terminal_barrier = {'native': {'epoch':10}}
            p.terminal_host = Mock(pid=10101); p.terminal_host.wait.return_value = 86
            p.runtime.services.processes = NS(children={'host-0':p.terminal_host},
                start=Mock(return_value=NS(pid=10102)))
            p.runtime.services.wait_ready = Mock(); p._configuration = Mock()
            stop = copy.deepcopy(p.action)
            stop['identity'].update(epoch=10, operation='op-0123456789abcdef-4-4')
            stop['command']['logical']['command'] = {'StopVm':dict(sandbox=4, epoch=10, generation=3, vm=10)}
            cleanup = copy.deepcopy(stop)
            cleanup['command']['logical']['command'] = {'CleanupVm':dict(sandbox=4, generation=3, vm=10)}
            prepared = self.receipt(p.action, 'Prepared', dict(pid=123, start_ticks=456, boot_id='boot'))
            stopped = self.receipt(stop, 'Stopped', dict(pid=123, start_ticks=456))
            snapshot = dict(local={'sandbox':4}, resume_actions=[(p.action, 'receipted', prepared)],
                terminal_actions=[(stop, 'receipted', stopped), (cleanup, 'dispatched', None)])
            def read(_deadline):
                self.assertTrue(p.terminal_host.wait.called, 'read must follow actual cut manager exit')
                return snapshot
            p._read_terminal = Mock(side_effect=read)
            return p, snapshot

        def test_cleanup_cut_freezes_original_receipts_before_one_read_then_original_recovery(self):
            p, _ = self.cleanup_cut_fixture()
            with patch.dict(globals(), pidfd_exited=lambda _fd:True, original_process=lambda _source:False):
                result = p.assert_terminal_before_gc(p.deadline)
            self.assertEqual(p._read_terminal.call_count, 1)
            self.assertTrue(result['original_prepare_observed'])
            self.assertTrue(result['original_stop_observed'])
            self.assertTrue(result['cleanup_sql_ack_pending_at_cut'])
            self.assertFalse(result['capacity_released'])
            p.runtime.services.processes.start.assert_called_once_with('host-0')

        def test_cleanup_cut_rejects_retired_rows_acknowledged_cleanup_or_wrong_candidate(self):
            for mutate in (
                    lambda snapshot: snapshot.update(resume_actions=[]),
                    lambda snapshot: snapshot['terminal_actions'].pop(),
                    lambda snapshot: snapshot['terminal_actions'][1][0]['command']['logical']['command']['CleanupVm'].update(vm=11),
                    lambda snapshot: snapshot['terminal_actions'][0][2]['outcome']['details']['result']['Stopped'].update(pid=124),
                    lambda snapshot: snapshot['terminal_actions'].__setitem__(1,
                        (snapshot['terminal_actions'][1][0], 'receipted', {}))):
                p, snapshot = self.cleanup_cut_fixture(); mutate(snapshot)
                with patch.dict(globals(), pidfd_exited=lambda _fd:True, original_process=lambda _source:False):
                    with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
                p.runtime.services.processes.start.assert_not_called()
            p, _ = self.cleanup_cut_fixture(); p.terminal_host.wait.return_value = 0
            with self.assertRaises(ValueError):p.assert_terminal_before_gc(p.deadline)
            p._read_terminal.assert_not_called()

    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(IdentityTests))
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    import sys
    if sys.argv[1:] != ['--self-test']:
        raise SystemExit('Import from the original lifecycle scenario; only --self-test runs standalone')
    raise SystemExit(self_test())
