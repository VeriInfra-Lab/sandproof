"""Finite consumers of original capture handoffs; never a VM supervisor.

Call arm only after record_running. During a held copy gate no helper may stop
or audit the Host. Public completion and product GC remain the caller's job.
An exit code, gate command or missing process alone never means Unloaded/Free.
"""
from contextlib import closing
import copy
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import time

from _center_host_services import role_facts
from lifecycle_unload_runtime import (
    checked_file, checked_source_records, continued_capture_question, gate_configuration,
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
}


class CaptureHandoffProbe:
    def __init__(self, runtime, client, physical_running, stage, gate_dir=None):
        if stage not in STAGES or physical_running['session'] != client.session:
            raise ValueError('one selected boundary and original public session required')
        self.runtime, self.client = runtime, client
        self.prior, self.stage = copy.deepcopy(physical_running), stage
        self.path = Path(gate_dir) if gate_dir is not None else None
        self.fd = self.pidfd = None
        self.files, self.accepted = {}, None
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
        if self.stage == 'restore-prepare-receipt':
            generation += 1  # gate_configuration rejects overflow; no old-generation pidfd adoption.
        native = runtime.services.native_pools[0]
        sandbox = 'sb-' + native['namespace'] + '-' + str(self.prior['local']['sandbox'])
        self.config = gate_configuration(runtime.lease.identity, sandbox, generation, timeout_ms)
        self.deadline = min(deadline, time.monotonic() + timeout_ms / 1000)
        if self.stage != 'restore-prepare-receipt':
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
        if self.stage == 'restore-prepare-receipt':
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
        if self.stage != 'restore-prepare-receipt' and fields['vm'] != self.prior['context']['binding']['vm']:
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
        expected_kind = 'resume' if self.stage == 'restore-prepare-receipt' else 'unload'
        if entry['intent']['request']['operation']['kind'] != expected_kind:
            raise ValueError('selected boundary has another original lifecycle operation')
        self.action = self._original_action(actions, entry)
        if self.stage == 'restore-prepare-receipt':
            candidates = [r for r in records if r['restore'] is not None
                          and r['restore']['intent'] == entry['intent']]
            if (len(candidates) != 1 or candidates[0]['restore']['activation'] is not None
                    or candidates[0]['restore']['material_use'] is not True):
                raise ValueError('original LoadPaused pin before activation marker required')
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

    def finish(self, deadline):
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

    def close(self):
        """Close observation descriptors only; uncertain effects and files remain."""
        for name in ('fd', 'pidfd', 'worker_pidfd'):
            descriptor = getattr(self, name, None)
            if descriptor is not None:
                os.close(descriptor)
                setattr(self, name, None)


def self_test():
    """The same four pure input checks; no services, Rust build or VM."""
    import unittest
    from types import SimpleNamespace as NS
    class IdentityTests(unittest.TestCase):
        def fixture(self,stage='freeze-receipt'):
            p=object.__new__(CaptureHandoffProbe)
            p.stage=stage;p.prior=dict(local=dict(sandbox=4,owner=7),context=dict(binding=dict(generation=2,vm=8)))
            p.config=dict(sandbox='sb-0123456789abcdef-4')
            p.runtime=NS(services=NS(native_pools=[dict(namespace='0123456789abcdef')]))
            command=STAGES[stage][1];generation=3 if stage=='restore-prepare-receipt' else 2
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
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(IdentityTests))
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    import sys
    if sys.argv[1:] != ['--self-test']:
        raise SystemExit('Import from the original lifecycle scenario; only --self-test runs standalone')
    raise SystemExit(self_test())
