"""E8 after the original source exited, before its Stop SQL receipt.

The cleanup cut may belong to the old Unload or to the later Kill. The center
can already be terminal while the Host is still finishing the old operation.
Neither legal order is a new activation or permission to revive the session.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

import copy
import json
from pathlib import Path

from _internal_fault_relay import _fields, _opaque, _u64, accepted_request
from lifecycle_unload_capture_faults import CaptureHandoffProbe
from lifecycle_unload_expiry import read_host
from lifecycle_unload_runtime import material_source_charge, original_json, original_process, pidfd_exited, remaining
from tools.check_kvm_host import unique_keys
from tools.template_build_guest import equal


class StoppedUnloadDestroyProbe(CaptureHandoffProbe):
    def __init__(self, runtime, client, physical_running):
        super().__init__(runtime, client, physical_running, 'stop-receipt')
        self.running_acceptance = copy.deepcopy(client.current_acceptance)
        self.before_destroy = self.cleanup_cut = None
        self.cleanup_host = None

    def _read(self, deadline, **kwargs):
        value = super()._read(deadline, **kwargs)
        self.last_snapshot = value
        return value

    def _exited(self):
        if self.pidfd is None or not pidfd_exited(self.pidfd):
            raise ValueError('positive original source pidfd exit required; absence is insufficient')
        # A zombie can retain its identity after kernel exit. Keep that fact;
        # the final product accounting/cleanup oracle still owns PID release.
        return dict(pidfd_exit_observed=True,
                    process_identity_present=original_process(self.prior['source']),
                    resource_release_inferred=False)

    @staticmethod
    def _action_shape(action):
        _fields(action, 'identity command')
        identity = _fields(action['identity'], 'protocol_version owner sandbox operation epoch generation logical_action attempt resource')
        for name in ('protocol_version', 'epoch', 'generation', 'attempt'):
            _u64(identity[name])
        for name in ('owner', 'sandbox', 'operation', 'logical_action'):
            _opaque(identity[name])
        resource = _fields(identity['resource'], 'kind id')
        _opaque(resource['id'])
        _fields(action['command'], 'attempt logical')
        _u64(action['command']['attempt'])
        _fields(action['command']['logical'], 'command')
        command = action['command']['logical']['command']
        if len(command) != 1:
            raise ValueError('one typed original command required')
        name = next(iter(command))
        fields = {'StopVm': 'sandbox epoch generation vm', 'CleanupVm': 'sandbox generation vm',
                  'InstallFence': 'sandbox epoch terminal'}
        if name not in fields:
            raise ValueError('one original Stop, cleanup or fence command required')
        values = _fields(command[name], fields[name])
        for key, value in values.items():
            if key == 'terminal':
                if type(value) is not bool:
                    raise ValueError('typed terminal bit required')
            else:
                _u64(value)
        if identity['attempt'] != action['command']['attempt']:
            raise ValueError('complete original attempt relation required')

    def _before_facts(self, snapshot):
        entry, records, actions = snapshot
        accepted_request(entry['intent'], self.runtime.services.host_configs[0]['inbox']['node'])
        if entry['intent']['request']['operation']['kind'] != 'unload' or entry['unload_resolution'] is not None:
            raise ValueError('original successful-source Unload, never a Continued result, required')
        stop = self._original_action(actions, entry)
        self._action_shape(stop)
        if not equal(stop, self.action):
            raise ValueError('original unacknowledged Stop changed')
        if (not equal(stop['identity']['resource'], dict(kind='vm', id=self.prior['source']['resource_id']))
                or not equal(self.record['work']['requirement']['resource'], stop['identity']['resource'])):
            raise ValueError('complete original source and Stop resource required')
        selected = [row for row in records if equal(row['work'], self.record['work'])]
        if len(selected) != 1 or not equal(selected[0]['capture'], self.record['capture']):
            raise ValueError('complete original adopted Full required before Destroy')
        source = selected[0]
        origin = source['work']['anchor']['execution_origin']
        bindings = origin['request']['binding']
        accepted_request(origin, self.runtime.services.host_configs[0]['inbox']['node'])
        if (not equal(dict(session=bindings['session'], key=bindings['key'], accepted_version=origin['accepted_version']), self.running_acceptance)
                or origin['request']['operation']['kind'] != 'resume'):
            raise ValueError('original current Resume origin required')
        backing = [row for row in records if row['restore'] is not None
                   and equal(row['restore']['intent'], origin)]
        if (len(backing) != 1 or backing[0]['restore']['material_use'] is not True
                or material_source_charge(backing[0]) <= 0 or material_source_charge(source) <= 0
                or not equal(backing[0]['capture']['completed'], origin['request']['operation']['manifest_digest'])):
            raise ValueError('original backing pin and both positive material duties required')
        resume = origin['request']['operation']
        previous = backing[0]['capture']['full']['pause']['binding']
        if (entry['intent']['request']['operation']['source_incarnation'] != resume['target_incarnation']
                or any(not equal(resume[name], previous[name]) for name in ('source_incarnation', 'capture'))):
            raise ValueError('complete original Resume source/target and both Full bindings required')
        return source, backing[0]

    def _registered_action(self, conn, action, capacity):
        self._action_shape(action)
        logical = action['identity']['logical_action']
        number = logical.removeprefix('effect-')
        if not number.isascii() or not number.isdecimal() or not 0 < int(number) < 2**63 or logical != 'effect-' + str(int(number)):
            raise ValueError('original canonical SQL logical action identifier required')
        size = conn.execute('SELECT length(logical_json) FROM controller_effect_keys WHERE id=?',(int(number),)).fetchone()
        if size is None or not 0 < size[0] <= capacity['max_sql_value_bytes']:
            raise ValueError('bounded original logical action row required')
        raw, = conn.execute('SELECT logical_json FROM controller_effect_keys WHERE id=?',(int(number),)).fetchone()
        if not equal(json.loads(raw,object_pairs_hook=unique_keys),action['command']['logical']):
            raise ValueError('complete original logical action differs from native registration')

    def _pending_commands(self, conn, capacity, command):
        count, largest = conn.execute("SELECT count(*),max(length(action_json)) FROM outbox WHERE dispatch_state='dispatched'").fetchone()
        if count > capacity['max_sql_rows'] or (largest is not None and largest > capacity['max_sql_value_bytes']):
            raise ValueError('bounded global pending actions required')
        actions = []
        for raw, in conn.execute("SELECT action_json FROM outbox WHERE dispatch_state='dispatched' ORDER BY id"):
            action = json.loads(raw, object_pairs_hook=unique_keys)
            if command in action['command']['logical']['command']:
                self._registered_action(conn, action, capacity)
                actions.append(action)
        return actions

    def _pending_stops(self, conn, local, capacity):
        return self._pending_commands(conn, capacity, 'StopVm')

    def _before_snapshot(self, deadline):
        value = self._read(deadline, extra=self._pending_stops)
        if not equal(value[3], [self.action]):
            raise ValueError('one global pending original Stop required to bind the exit86 cut')
        return value[:3]

    def after_acceptance(self, accepted, deadline):
        deadline = min(deadline, self.deadline)
        result = super().after_acceptance(accepted, deadline)
        exit_observation = self._exited()
        source, backing = self._before_facts(self._before_snapshot(deadline))
        # The parent just checked all B bytes against this exact unchanged
        # Full. Reuse that read; both bundles are reread before reopening.
        materials = copy.deepcopy(self.materials)
        old_materials = self.runtime._materials(backing['capture']['full'], backing['capture'])
        if not equal(old_materials, self.prior['running_backing']):
            raise ValueError('original complete material bytes changed before Destroy')
        remaining(deadline)
        self.original_entry = copy.deepcopy(self.last_snapshot[0])
        self.before_destroy = dict(original_stop=copy.deepcopy(self.action), original_source_exit=True,
            source_exit_observation=exit_observation,
            global_pending_stop_matched=True,
            original_stop_sql_receipt_pending=True, source_materials=materials, backing_materials=old_materials,
            source_charge=material_source_charge(source), backing_charge=material_source_charge(backing),
            public_completion=False)
        # Bookkeeping uses paths already read and validated by the original
        # material reader; it neither deletes files nor manufactures GC facts.
        self.runtime.material_paths.setdefault(self.client.session, set()).update(materials['material_paths'])
        return dict(result, stopped_before_destroy=self.before_destroy)

    def _extra(self, conn, local, capacity):
        # The parent already checked this same transaction's inbox bound and
        # checksum. Read it again in that transaction to bind the actual order.
        raw, = conn.execute('SELECT state_json FROM controller_lifecycle_inbox WHERE id=1').fetchone()
        inbox = json.loads(raw, object_pairs_hook=unique_keys)
        sessions = [row for row in inbox['sessions'] if row['id'] == self.accepted['session']]
        if len(sessions) != 1:
            raise ValueError('one original current Host session required')
        session = sessions[0]
        if session['current_version'] == self.accepted['accepted_version']:
            if session['terminal'] is not False:
                raise ValueError('old Unload has an unexpected terminal bit')
            terminal, terminal_actions = None, []
        else:
            _, terminal, _ = self._terminal_entries_for(inbox, self.original_entry, require_admitted=False)
            accepted_request(terminal['intent'], self.runtime.services.host_configs[0]['inbox']['node'])
            terminal_actions = ([] if terminal['local_operation'] is None else
                                self._terminal_outbox(conn, local, terminal['local_operation'], capacity))
        original_actions = self._terminal_outbox(conn, local, self.original_entry['local_operation'], capacity)
        for action, _, _ in original_actions + terminal_actions:
            if any(name in action['command']['logical']['command'] for name in ('StopVm','CleanupVm','InstallFence')):
                self._registered_action(conn, action, capacity)
        return dict(current_version=session['current_version'], terminal=terminal,
            terminal_actions=terminal_actions, native=self._read_native(conn, local, capacity),
            global_pending_cleanups=self._pending_commands(conn, capacity, 'CleanupVm'))

    def _cleanup_facts(self, snapshot):
        entry, _, original_actions, extra = snapshot
        for name in ('owner', 'generation', 'epoch'):
            _u64(extra['native'][name])
        if (extra['native']['owner'] != self.prior['local']['owner']
                or extra['native']['generation'] != self.action['identity']['generation']
                or extra['native']['epoch'] < self.action['identity']['epoch']):
            raise ValueError('original native owner and generation required at cleanup cut')
        if not equal(entry, self.original_entry):
            raise ValueError('historical original Unload entry changed after Destroy')
        old = [row for row in original_actions if equal(row[0], self.action)]
        if len(old) != 1 or old[0][1] not in ('dispatched', 'receipted'):
            raise ValueError('one retained original Stop required at the fixed cleanup cut')
        terminal_actions = extra['terminal_actions']
        rows = original_actions + terminal_actions
        if any(any(name in row[0]['command']['logical']['command'] for name in ('PrepareVm', 'ActivateVm')) for row in rows):
            raise ValueError('Unload/Destroy must not prepare or activate a replacement')
        cleanups = [row for row in rows if 'CleanupVm' in row[0]['command']['logical']['command']
                    and row[1:] == ('dispatched', None)]
        if len(cleanups) != 1:
            raise ValueError('one original Cleanup-success/pre-SQL receipt required')
        cleanup = cleanups[0][0]
        if not equal(extra['global_pending_cleanups'], [cleanup]):
            raise ValueError('one global pending original Cleanup required to bind the exit86 cut')
        self._action_shape(cleanup)
        original = self.action['command']['logical']['command']['StopVm']
        expected = {key: original[key] for key in ('sandbox', 'generation', 'vm')}
        if (not equal(cleanup['command']['logical']['command'], dict(CleanupVm=expected))
                or not equal(cleanup['identity']['resource'], self.action['identity']['resource'])):
            raise ValueError('cleanup cut names another original VM or generation')
        own_cleanup = any(equal(row[0], cleanup) for row in original_actions)
        for name in ('owner', 'sandbox', 'generation', 'protocol_version'):
            if not equal(cleanup['identity'][name], self.action['identity'][name]):
                raise ValueError('cleanup has another complete original owner or action binding')
        if own_cleanup:
            if cleanup['identity']['operation'] != self.action['identity']['operation']:
                raise ValueError('original cleanup names another Unload operation')
            stopped = self._success_observation(old[0], 'Stopped')
            epoch = self.action['identity']['epoch']
            order = 'old-unload-cleanup'
        else:
            if extra['terminal'] is None:
                raise ValueError('terminal cleanup requires the actual delivered Destroy')
            if (extra['native']['terminal'] is not True
                    or not equal(extra['native']['generation'], self.action['identity']['generation'])):
                raise ValueError('same generation and actual native terminal decision required')
            if any('CommitSnapshot' in row[0]['command']['logical']['command'] for row in terminal_actions):
                raise ValueError('current Destroy must not commit a new snapshot')
            epoch = extra['native']['epoch']
            stops = [row for row in terminal_actions if 'StopVm' in row[0]['command']['logical']['command']]
            fences = [row for row in terminal_actions if 'InstallFence' in row[0]['command']['logical']['command']]
            if len(stops) != 1 or len(fences) != 1:
                raise ValueError('one actual terminal fence and same-source Stop required')
            stop = stops[0][0]
            self._action_shape(stop)
            self._action_shape(fences[0][0])
            if (not equal(stop['command']['logical']['command'], dict(StopVm=dict(expected, epoch=epoch)))
                    or not equal(stop['identity']['resource'], self.action['identity']['resource'])
                    or stop['identity']['epoch'] != epoch):
                raise ValueError('terminal Stop names another original source')
            observed = self._success_observation(fences[0], 'Fence')
            if (not equal(observed, dict(epoch=epoch, terminal=True))
                    or fences[0][0]['identity']['epoch'] != epoch
                    or not equal(fences[0][0]['identity']['resource'],dict(kind='sandbox',id=self.config['sandbox']))
                    or not equal(fences[0][0]['command']['logical']['command'],
                                 dict(InstallFence=dict(sandbox=expected['sandbox'], epoch=epoch, terminal=True)))):
                raise ValueError('original terminal fence is not confirmed')
            path = Path(self.runtime.services.native_pools[0]['node_state_dir']) / 'fences' / self.config['sandbox']
            if not equal(original_json(path.with_suffix('.json'), 16384), observed):
                raise ValueError('actual terminal fence differs from its receipt')
            stopped = self._success_observation(stops[0], 'Stopped')
            # The old receipt may still be unknown after terminal takeover.
            # Any claimed successful old Stop must name the same dead source.
            old_stopped = self._success_observation(old[0], 'Stopped')
            if old_stopped is not None and not equal(old_stopped, stopped):
                raise ValueError('old Stop and terminal Stop name different sources')
            order = 'terminal-kill-cleanup'
        _u64(epoch)
        if (cleanup['identity']['epoch'] != epoch
                or not equal(stopped, dict(pid=self.prior['source']['pid'], start_ticks=self.prior['source']['start_time_ticks']))):
            raise ValueError('cleanup and observed Stop differ from the original exited source')
        exit_observation = self._exited()
        return dict(order=order, original_stop=copy.deepcopy(self.action),
            cleanup=copy.deepcopy(cleanup), stop_observation=copy.deepcopy(stopped),
            host_current_version=extra['current_version'], host_destroy_delivered=extra['terminal'] is not None,
            cleanup_sql_ack_pending=True, original_source_exit=True, public_completion=False,
            source_exit_observation=exit_observation,
            capacity_release_inferred=False)

    def recover_after_destroy(self, accepted, deadline):
        if self.before_destroy is None or self.terminal_accepted is not None or self.recovered or self.host.poll() != 86:
            raise ValueError('one original exited source and public Destroy required')
        deadline = min(deadline, self.deadline)
        self._exited()
        self._bind_next_destroy(accepted)
        source, backing = self._before_facts(self._before_snapshot(deadline))
        for row, name in ((source, 'source_materials'), (backing, 'backing_materials')):
            if not equal(self.runtime._materials(row['capture']['full'], row['capture']), self.before_destroy[name]):
                raise ValueError('material responsibility changed before original Host reopen')
        self._configuration()
        self.cleanup_host = self.runtime.services.processes.start('host-0', env={'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'cleanup_vm'})
        if self.cleanup_host is self.host or self.cleanup_host.pid == self.host.pid:
            raise ValueError('fresh original manager required after Stop cut')
        if self.cleanup_host.wait(timeout=remaining(deadline)) != 86:
            raise ValueError('original manager did not reach Cleanup-success/pre-SQL cut')
        self._configuration()
        current = read_host(self.runtime, self.accepted['session'], deadline)['current_version']
        if type(current) is not int or current not in (self.accepted['accepted_version'], self.terminal_accepted['accepted_version']):
            raise ValueError('unrelated current Host version at the original cleanup cut')
        cut = self._cleanup_facts(self._read(deadline, current_version=current, extra=self._extra))
        self.cleanup_cut = copy.deepcopy(cut)
        self.cleanup_cut['terminal_replay'] = self.client.assert_terminal_unload_replay(phase='terminating')
        remaining(deadline)
        process = self.runtime.services.processes.start('host-0')
        if process is self.cleanup_host or process.pid == self.cleanup_host.pid:
            raise ValueError('fresh original manager required after cleanup cut')
        self.runtime.services.wait_ready(timeout=min(remaining(deadline), 900), for_admission=False)
        self._configuration()
        remaining(deadline)
        self.recovered = True
        return dict(self.cleanup_cut, original_recovery_resumed=True)

    def finish(self, deadline):
        raise ValueError('this Destroy branch must not wait for the old Unloaded result')

    def cleanup(self):
        if self.cleanup_cut is None or not self.recovered or not self.runtime.gc_complete or not self.runtime.physical_complete:
            raise ValueError('original cleanup cut and complete product GC required')
        return dict(original_cleanup_cut_observed=True, product_gc_complete=True)

    def terminal_failure_state(self):
        return dict(original_acceptance=copy.deepcopy(self.accepted), destroy_acceptance=copy.deepcopy(self.terminal_accepted),
            before_destroy=copy.deepcopy(self.before_destroy), cleanup_cut=copy.deepcopy(self.cleanup_cut),
            action='none; preserve original accepted operations and unknown resource/material duties',
            public_completion_claimed=False, capacity_release_inferred=False)
