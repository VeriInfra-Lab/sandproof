"""Finite E5 pre-Stop observations followed by the original lost-receipt path.

The generic socket carries no action identity. Its original Host peer, one
global in-flight Stop in the SAME SQL snapshot, and the exact paused VMM must
all agree before sending g. Sending g is not a Stop/exit/Unloaded receipt.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

import copy
import json
import os
import socket
import stat
import struct
import time

from _internal_fault_relay import _fields, _opaque, _u64, accepted_request
from lifecycle_freeze_destroy import original_backing, paused_source, pending_stop_facts
from lifecycle_unload_capture_faults import CaptureHandoffProbe
from lifecycle_unload_runtime import material_source_charge, original_process, pidfd_exited, remaining
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path
from tools.template_build_guest import equal


class PendingStopProbe(CaptureHandoffProbe):
    def __init__(self, runtime, client, physical_running):
        super().__init__(runtime, client, physical_running, 'stop-receipt')
        self.running_acceptance = copy.deepcopy(client.current_acceptance)
        self.stop_path = private_path(runtime.quota['deployment_control'], directory=True) / 'unload-stop.sock'
        if self.stop_path.exists() or self.stop_path.is_symlink() or len(os.fsencode(self.stop_path)) >= 108:
            raise ValueError('fresh bounded original Stop socket required')
        self.stop_peer = None
        self.stop_released = False
        self.pending = None

    def _fault_environment(self):
        fault = super()._fault_environment()
        if fault != {'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'stop_vm'}:
            raise ValueError('original Stop-success/pre-SQL exit required')
        return dict(fault, SANDPROOF_TEST_STOP_GATE=str(self.stop_path))

    def _extra(self, conn, local, capacity):
        facts = pending_stop_facts(self.runtime, conn, local, capacity)
        name = 'sb-' + self.runtime.services.native_pools[0]['namespace'] + '-' + str(local['sandbox'])
        row = conn.execute('SELECT length(state_blob) FROM sandboxes WHERE sandbox_id=?', (name,)).fetchone()
        if row is None or not 0 < row[0] <= capacity['max_sql_value_bytes']:
            raise ValueError('bounded original native row required')
        raw, owner = conn.execute('SELECT state_blob,owner_id FROM sandboxes WHERE sandbox_id=?', (name,)).fetchone()
        facts['native'] = json.loads(raw, object_pairs_hook=unique_keys)
        if owner != 'tenant-' + str(local['owner']):
            raise ValueError('original native owner required')
        return facts

    @staticmethod
    def _stop_shape(stop):
        _fields(stop, 'identity command')
        identity = _fields(stop['identity'], 'protocol_version owner sandbox operation epoch generation logical_action attempt resource')
        for key in ('protocol_version', 'epoch', 'generation', 'attempt'):
            _u64(identity[key])
        for key in ('owner', 'sandbox', 'operation', 'logical_action'):
            _opaque(identity[key])
        resource = _fields(identity['resource'], 'kind id')
        _opaque(resource['id'])
        command = _fields(stop['command'], 'attempt logical')
        _u64(command['attempt'])
        logical = _fields(command['logical'], 'command')
        _fields(logical['command'], 'StopVm')
        fields = _fields(logical['command']['StopVm'], 'sandbox epoch generation vm')
        for value in fields.values():
            _u64(value)

    def _snapshot(self, deadline):
        return self._read(deadline, extra=self._extra)

    def _facts(self, snapshot):
        entry, rows, actions, extra = snapshot
        node = self.runtime.services.host_configs[0]['inbox']['node']
        intent = entry['intent']
        accepted_request(intent, node)
        binding = intent['request']['binding']
        request = self.client.unload_request
        if (intent['request']['operation']['kind'] != 'unload'
                or not equal(dict(session=binding['session'], key=binding['key'],
                                  accepted_version=intent['accepted_version']), self.accepted)
                or not equal(binding['expected'], dict(state='version', version=self.running_acceptance['accepted_version']))
                or not equal(request, dict(window=binding['key']['window'], sequence=binding['key']['sequence'],
                                           expected_version=self.running_acceptance['accepted_version']))
                or intent['accepted_version'] != self.running_acceptance['accepted_version'] + 1
                or entry['unload_resolution'] is not None):
            raise ValueError('original next pending Unload and complete public request required')
        stop = self._original_action(actions, entry)
        self._stop_shape(stop)
        native = extra['native']
        source = self.prior['source']
        current = self.prior['context']['binding']
        if (not equal(extra['global_pending_stops'], [stop]) or extra['logical_action_registered'] is not True
                or not equal(stop['identity']['resource'], dict(kind='vm', id=source['resource_id']))
                or native['terminal'] is not False
                or not equal({key: native[key] for key in ('owner', 'generation', 'current_vm', 'epoch')},
                             dict(owner=current['owner'], generation=current['generation'],
                                  current_vm=current['vm'], epoch=stop['identity']['epoch']))
                or not equal(extra['lifetime'], self.prior['lifetime'])):
            raise ValueError('one global original Stop, live native identity and immutable lifetime required')
        if any('CleanupVm' in action['command']['logical']['command']
               and action['command']['logical']['command']['CleanupVm']['vm'] == current['vm']
               for action, _, _ in actions):
            raise ValueError('original VMM cleanup appeared before its physical Stop')
        selected = [row for row in rows if equal(row['work']['anchor']['request'], intent)]
        if len(selected) != 1:
            raise ValueError('one original pending Unload source required')
        self.record = selected[0]
        requirement = self.record['work']['requirement']
        if (not equal(requirement['source'], {key: source[key] for key in ('pid', 'start_time_ticks', 'boot_id')})
                or not equal(requirement['resource'], stop['identity']['resource'])
                or not equal(requirement['node'], node)):
            raise ValueError('original source changed before Stop')
        captured = self._full(entry)
        backing = original_backing(self.runtime, self.running_acceptance, rows)
        if (captured['completed'] is None or material_source_charge(self.record) <= 0
                or intent['request']['operation']['source_incarnation'] !=
                backing['restore']['intent']['request']['operation']['target_incarnation']):
            raise ValueError('adopted original Full, positive charge and current source incarnation required')
        return dict(stop=copy.deepcopy(stop), entry=copy.deepcopy(entry),
                    record=copy.deepcopy(self.record), backing=copy.deepcopy(backing))

    def _live(self, deadline):
        if (self.host.poll() is not None or self.pidfd is None or pidfd_exited(self.pidfd)
                or not original_process(self.prior['source'])):
            raise ValueError('original Host and exact source must remain live before Stop release')
        return paused_source(self.runtime, self.prior, deadline)

    def _public_pending(self):
        view = self.client.query()
        self.client.check_view(view, self.accepted)
        if view['phase'] != 'unloading' or view['unload_outcome'] != {'state': 'pending'}:
            raise ValueError('public Unloaded/terminal/loss cannot precede physical Stop')
        self.client.assert_data_closed()
        self.client.assert_previous_tokens_closed()
        return copy.deepcopy(view)

    def _gate_identity(self):
        meta = self.stop_path.lstat()
        role = self.runtime.services.processes.bindings['host-0']
        if not stat.S_ISSOCK(meta.st_mode) or meta.st_uid != role['uid']:
            raise ValueError('original Host Stop socket required')
        return meta.st_dev, meta.st_ino

    def _before_observation(self, deadline):
        if self.pending is not None or self.stop_peer is not None or self.stop_released:
            raise ValueError('one pre-Stop observation required')
        while not self.stop_path.exists():
            if self.host.poll() is not None:
                raise ValueError('Host exited before original pre-Stop gate')
            time.sleep(min(.05, remaining(deadline)))
        identity = self._gate_identity()
        role = self.runtime.services.processes.bindings['host-0']
        peer = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.stop_peer = peer
        peer.settimeout(min(2, remaining(deadline)))
        peer.connect(str(self.stop_path))
        if struct.unpack('3i', peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)) != (
                self.host.pid, role['uid'], role['gid']):
            raise ValueError('Stop socket peer differs from original Host')
        self._configuration()
        initial = self._facts(self._snapshot(deadline))
        paused = self._live(deadline)
        materials = self.runtime._materials(initial['record']['capture']['full'], initial['record']['capture'])
        backing = self.runtime._materials(initial['backing']['capture']['full'], initial['backing']['capture'])
        if not equal(backing, self.prior['running_backing']):
            raise ValueError('original running backing bytes changed before Stop')
        public = self._public_pending()
        again = self._facts(self._snapshot(deadline))
        # Fresh observations/producer-End fields are not physical-exit evidence.
        # The work, producer, adopted digest and backing duties stay exact.
        for name in ('stop', 'entry'):
            if not equal(initial[name], again[name]):
                raise ValueError('original pending Stop or accepted entry changed')
        for name in ('record', 'backing'):
            old, new = initial[name], again[name]
            if (not equal(old['work'], new['work'])
                    or any(not equal(old['capture'][key], new['capture'][key]) for key in
                           ('full', 'action', 'worker', 'completed'))
                    or material_source_charge(old) != material_source_charge(new)):
                raise ValueError('original material identity or charge changed while Stop held')
        if (not equal(materials, self.runtime._materials(again['record']['capture']['full'], again['record']['capture']))
                or not equal(backing, self.runtime._materials(again['backing']['capture']['full'], again['backing']['capture']))):
            raise ValueError('original complete material bytes changed while Stop held')
        self._live(deadline)
        public_after = self._public_pending()
        if not equal(public, public_after) or self._gate_identity() != identity:
            raise ValueError('original pending public state or Stop gate changed')
        self.pending = dict(original_stop=initial['stop'], public_before=public, public_after=public_after,
                            source=paused, source_still_live=True, materials=materials,
                            original_backing=backing, material_charge=material_source_charge(again['record']),
                            backing_charge=material_source_charge(again['backing']),
                            same_sql_snapshot_for_global_stop=True, public_completion=False)
        remaining(deadline)
        peer.sendall(b'g')
        self.stop_released = True
        peer.close()
        self.stop_peer = None

    def after_acceptance(self, accepted, deadline):
        result = super().after_acceptance(accepted, deadline)
        if self.pending is None or not self.stop_released or not equal(self.pending['original_stop'], self.action):
            raise ValueError('lost Stop receipt differs from original held Stop')
        return dict(result, before_physical_stop=self.pending, stop_gate_release_sent=True,
                    original_stop_sql_receipt_pending=True, original_source_exit_observed=True)

    def failure_state(self):
        return dict(original_acceptance=copy.deepcopy(self.accepted), stop_release_sent=self.stop_released,
                    pre_stop_observation=copy.deepcopy(self.pending), public_completion=False,
                    responsibility='preserve original accepted source and material duties; no automatic release or new request')

    def cleanup(self):
        if (not self.recovered or not self.stop_released or not self.runtime.gc_complete
                or not self.runtime.physical_complete or self.stop_path.exists() or self.stop_path.is_symlink()):
            raise ValueError('original Stop/reopen and product GC required; never unlink an unknown socket')
        return dict(original_consumed_stop_socket_absent=True)

    def close(self):
        # Closing an observation peer can leave the original manager's gate
        # read unknown/failed. It never sends g or establishes physical exit.
        closed = self.stop_peer is not None
        if closed:
            self.stop_peer.close()
            self.stop_peer = None
        super().close()
        return dict(observation_peer_closed=closed, stop_release_sent=self.stop_released,
                    physical_exit_inferred=False, unknown_responsibilities_preserved=True)
