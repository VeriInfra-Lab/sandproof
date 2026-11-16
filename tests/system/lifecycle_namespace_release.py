"""Original Stop/reopen with a real reference to the old network namespace.

The descriptor supplies no exit/release evidence. Existing Rust and product GC
must finish the original binding after the observer explicitly closes it.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)["require_assertions"]
_require_assertions()

import copy
import fcntl
import json
import os
from pathlib import Path
import re
import stat
import time

from _internal_fault_relay import _fields, _u64
from lifecycle_freeze_destroy import original_backing
from lifecycle_unload_runtime import (checked_material_release, material_source_charge,
    original_json, original_process, pidfd_exited, remaining)
from lifecycle_unload_stop_pending import PendingStopProbe
from tools.check_kvm_host import unique_keys
from tools.template_build_guest import equal


def open_namespace_file(path):
    before = path.lstat()
    if (not stat.S_ISREG(before.st_mode) or before.st_uid != 0
            or before.st_mode & 0o022):
        raise ValueError('original root-owned named namespace required')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        actual, after = os.fstat(fd), path.lstat()
        if any((value.st_dev, value.st_ino) != (before.st_dev, before.st_ino)
               for value in (actual, after)):
            raise ValueError('named namespace changed during open')
        return fd
    except BaseException:
        os.close(fd)
        raise


class NamespaceReference:
    def __init__(self, runtime, prior, pidfd):
        self.fd = None
        self.explicitly_closed = False
        source, binding = prior['source'], prior['context']['binding']
        for value in binding.values():
            _u64(value)
        namespace = runtime.services.native_pools[0]['namespace']
        name = 'vm-' + namespace + '-' + str(binding['vm'])
        if (source['resource_id'] != name or pidfd_exited(pidfd)
                or not original_process(source)):
            raise ValueError('same live original source required before holding its namespace')
        record = original_json(Path(runtime.services.native_pools[0]['chroot_base']) /
                               '.claims' / name / 'network.json', 16384)
        _fields(record, 'resource_id namespace_device namespace_inode')
        fd = open_namespace_file(Path('/run/netns') / name)
        try:
            meta = os.fstat(fd)
            process_namespace = os.stat('/proc/' + str(source['pid']) + '/ns/net')
            if (not equal(record, dict(resource_id=name, namespace_device=meta.st_dev,
                                      namespace_inode=meta.st_ino))
                    or (process_namespace.st_dev, process_namespace.st_ino) != (meta.st_dev, meta.st_ino)
                    or not fcntl.fcntl(fd, fcntl.F_GETFD) & fcntl.FD_CLOEXEC
                    or pidfd_exited(pidfd) or not original_process(source)):
                raise ValueError('original protected network, proc namespace and descriptor must agree')
            self.identity = dict(name=name, device=meta.st_dev, inode=meta.st_ino)
            self.fd = fd
        except BaseException:
            os.close(fd)
            raise

    def verify(self):
        if self.fd is None:
            raise ValueError('original namespace reference is no longer held')
        meta = os.fstat(self.fd)
        if (meta.st_dev, meta.st_ino) != (self.identity['device'], self.identity['inode']):
            raise ValueError('original namespace descriptor changed')
        return copy.deepcopy(self.identity)

    def close(self, *, explicit=False):
        if self.fd is not None:
            self.verify()
            os.close(self.fd)
            self.fd = None
            self.explicitly_closed = explicit


class NamespaceReleaseProbe(PendingStopProbe):
    def __init__(self, runtime, client, physical_running, *, observe_peer):
        super().__init__(runtime, client, physical_running)
        if not callable(observe_peer):
            raise ValueError('original other-owner workload observer required')
        self.observe_peer = observe_peer
        self.namespace_reference = None
        self.namespace_checks = []
        self.namespace_release = None
        self.unload_entry = None

    def _facts(self, snapshot):
        facts = super()._facts(snapshot)
        if self.unload_entry is not None and not equal(facts['entry'], self.unload_entry):
            raise ValueError('original pending Unload changed before physical Stop')
        self.unload_entry = copy.deepcopy(facts['entry'])
        return facts

    def arm(self, deadline, timeout_ms):
        result = super().arm(deadline, timeout_ms)
        self.namespace_reference = NamespaceReference(self.runtime, self.prior, self.pidfd)
        return dict(result, held_namespace=self.namespace_reference.verify(),
                    namespace_descriptor_inheritable=False)

    def after_acceptance(self, accepted, deadline):
        result = super().after_acceptance(accepted, deadline)
        return dict(result, old_namespace_reference=self.namespace_reference.verify(),
                    namespace_reference_survives_original_vmm_exit=True)

    def _witness(self, conn, local, capacity):
        name = 'sb-' + self.runtime.services.native_pools[0]['namespace'] + '-' + str(local['sandbox'])
        size = conn.execute('SELECT length(witness_json) FROM controller_release_witnesses '
                            'WHERE sandbox_id=?', (name,)).fetchone()
        if size is None:
            return None
        if not 0 < size[0] <= capacity['max_sql_value_bytes']:
            raise ValueError('bounded original release witness required')
        raw, = conn.execute('SELECT witness_json FROM controller_release_witnesses '
                            'WHERE sandbox_id=?', (name,)).fetchone()
        witness = json.loads(raw, object_pairs_hook=unique_keys)
        checked_material_release(dict(producer=witness, consumer=None))
        context = witness['context']
        expected = self.prior['context']
        if equal(context, expected):
            raise ValueError('original position was confirmed released while namespace descriptor remained held')
        if (context['binding']['owner'] != expected['binding']['owner']
                or context['binding']['sandbox'] != expected['binding']['sandbox']
                or context['binding']['generation'] >= expected['binding']['generation']):
            raise ValueError('foreign or nonolder release witness while original namespace held')
        return witness

    def _held_extra(self, conn, local, capacity):
        # Reuse the original bounded native/lifetime reader in this SAME SQL
        # snapshot. A terminal path has a different release/diagnostic owner.
        extra = super()._extra(conn, local, capacity)
        extra['release_witness'] = self._witness(conn, local, capacity)
        return extra

    def _held_state(self, deadline):
        self.namespace_reference.verify()
        if not pidfd_exited(self.pidfd):
            raise ValueError('the original VMM must have a positive kernel exit observation')
        entry, records, _, extra = self._read(deadline, extra=self._held_extra)
        if (not equal(entry['intent'], self.unload_entry['intent'])
                or entry['unload_resolution'] is not None):
            raise ValueError('held namespace requires the same pending original Unload')
        native = extra['native']
        expected = dict(owner=self.prior['local']['owner'], terminal=False,
            generation=self.prior['context']['binding']['generation'], epoch=self.action['identity']['epoch'])
        operation = _fields(native['current_operation'], 'id kind epoch status')
        if (not equal({name: native[name] for name in expected}, expected)
                or not equal({name: operation[name] for name in ('id', 'kind', 'epoch')},
                    dict(id=entry['local_operation'], kind='Pause', epoch=self.action['identity']['epoch']))
                or operation['status'] not in ('Pending', 'OutcomeUnknown', 'Succeeded')
                or native['phase'] not in ('Pausing', 'Paused')
                or (native['phase'] == 'Paused' and (native['current_vm'] is not None
                    or native['net_slot'] is not None or operation['status'] != 'Succeeded'))
                or not equal(extra['lifetime'], self.prior['lifetime'])):
            raise ValueError('same nonterminal native Pause and original lifetime required')
        selected = [record for record in records if equal(record['work'], self.record['work'])]
        if len(selected) != 1 or material_source_charge(selected[0]) <= 0:
            raise ValueError('original complete source and positive material responsibility required')
        for field in ('full', 'action', 'worker', 'completed'):
            if not equal(selected[0]['capture'][field], self.record['capture'][field]):
                raise ValueError('original adopted Full identity changed while namespace held')
        backing = original_backing(self.runtime, self.running_acceptance, records)
        position = self.prior['context']['position']
        pool = self.runtime.services.native_pools[0]
        stored = original_json(Path(pool['node_state_dir']) / 'host-slots' / (str(position) + '.json'),
                               self.runtime.services.host_configs[0]['native']['maintenance']['capacity']['max_sql_value_bytes'])
        _fields(stored, 'position helper_gate value')
        row = _fields(stored['value'], 'context phase retirement')
        if (type(stored['position']) is not int or stored['position'] != position
                or not equal(row['context'], self.prior['context'])
                or row['phase'] not in ('Occupied', 'Releasing')):
            raise ValueError('original node position was released or rebound while namespace held')
        return dict(pending_original_intent=True, node_phase=row['phase'],
                    native_phase=native['phase'],
                    material_charge=material_source_charge(selected[0]),
                    backing_charge=material_source_charge(backing),
                    previous_release_witness=extra['release_witness'], original_vmm_exited=True,
                    held_namespace=self.namespace_reference.verify(), public_completion=False)

    def _log_mark(self):
        path = self.runtime.services.processes.logs / 'host-0.log'
        info = path.lstat()
        role = self.runtime.services.processes.bindings['host-0']
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != role['uid']
                or info.st_nlink != 1 or info.st_mode & 0o022):
            raise ValueError('original Host diagnostic log required')
        return dict(path=path, device=info.st_dev, inode=info.st_ino, offset=info.st_size)

    def _faults(self, mark):
        fd = os.open(mark['path'], os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            info = os.fstat(fd)
            if ((info.st_dev, info.st_ino) != (mark['device'], mark['inode'])
                    or info.st_size < mark['offset'] or info.st_size - mark['offset'] > 1024 * 1024):
                raise ValueError('Host log replaced, truncated or beyond the original bounded observation')
            os.lseek(fd, mark['offset'], os.SEEK_SET)
            size = info.st_size - mark['offset']
            raw = bytearray()
            while len(raw) < size:
                block = os.read(fd, size - len(raw))
                if not block:
                    raise ValueError('Host log truncated during observation')
                raw.extend(block)
        finally:
            os.close(fd)
        binding = self.prior['context']['binding']
        pattern = (r'^controller_fault scope=(?:Instance|Host) stage=ReleaseSlot '
                   r'kind=(IdentityMismatch|UnresolvedAction) code=None io_kind=None '
                   r'sandbox=Some\(' + str(binding['sandbox']) + r'\) operation=None '
                   r'generation=Some\(' + str(binding['generation']) + r'\) attempt=None '
                   r'vm=Some\(' + str(binding['vm']) + r'\) epoch=None command=None$')
        # Do not accept an unfinished trailing write as a complete fault event.
        complete = raw[:raw.rfind(b'\n') + 1]
        return [match[1] for line in complete.decode('utf-8').splitlines()
                if (match := re.fullmatch(pattern, line))]

    def _wait_attempt(self, mark, deadline, *, completed_parent):
        # Normal Unload uses release_current_slot: Node Busy propagates out of
        # advance_current_workflow, with no journal UnresolvedAction diagnostic.
        # ActiveSandbox serializes that whole call. Two distinct matching Node
        # failures after this manager's mark prove the earlier call returned;
        # rereading one physical line never counts as a second attempt.
        required = 2 if completed_parent else 1
        while True:
            remaining(deadline)
            state = self._held_state(deadline)
            observed = self._faults(mark).count('IdentityMismatch')
            if observed >= required:
                state = self._held_state(deadline)
                if state['native_phase'] != 'Paused':
                    raise ValueError('original completed native Pause required at the actual release attempt')
                return dict(state, actual_host_release_attempt=True,
                            matching_node_failures=observed,
                            previous_release_attempt_returned=completed_parent)
            time.sleep(min(.05, remaining(deadline)))

    def _audit_held(self, deadline):
        old = self.runtime.services.processes.children['host-0']
        marks = []
        def observe(audit, local, lifetime, conn):
            if (not equal({key: local[key] for key in ('owner', 'sandbox')},
                          {key: self.prior['local'][key] for key in ('owner', 'sandbox')})
                    or not equal(lifetime, self.prior['lifetime'])
                    or not equal([(context, phase) for context, phase in audit['host_slots']
                        if equal(context, self.prior['context'])], [(self.prior['context'], 'Releasing')])):
                raise ValueError('actual journal must retain the original Releasing position')
            self._witness(conn, local,
                self.runtime.services.host_configs[0]['native']['maintenance']['capacity'])
            self.namespace_reference.verify()
            marks.append(self._log_mark())
            return dict(original_sql_phase='Releasing', original_lifetime_unchanged=True)
        result = self.runtime._host_snapshot(self.client.session, deadline, observe)
        current = self.runtime.services.processes.children['host-0']
        if old.poll() is None or current is old or current.pid == old.pid or len(marks) != 1:
            raise ValueError('one original manager exit and a fresh same-configuration manager required')
        self._configuration()
        self._held_state(deadline)
        return result, marks[0]

    def finish(self, deadline):
        deadline = min(deadline, self.deadline)
        mark = self._log_mark()
        recovered = super().finish(deadline)
        self.namespace_checks.append(self._wait_attempt(mark, deadline, completed_parent=False))
        first, mark = self._audit_held(deadline)
        self.namespace_checks.append(self._wait_attempt(mark, deadline, completed_parent=True))
        second, _ = self._audit_held(deadline)
        peer = self.observe_peer()
        state = self._held_state(deadline)
        self.namespace_reference.close(explicit=True)
        self.namespace_release = dict(recovered=recovered, before_reopen=first, after_reopen=second,
            held_checks=copy.deepcopy(self.namespace_checks), other_owner=peer,
            namespace_before_explicit_close=state['held_namespace'], namespace_descriptor_closed=True,
            public_completion=False, capacity_released=False)
        return copy.deepcopy(self.namespace_release)

    def cleanup(self):
        if (self.namespace_release is None or self.namespace_reference.fd is not None
                or not self.namespace_reference.explicitly_closed):
            raise ValueError('observed original rejection/reopen and explicit namespace close required')
        return dict(super().cleanup(), namespace_reference_closed=True)

    def failure_state(self):
        return dict(super().failure_state(), namespace_checks=copy.deepcopy(self.namespace_checks),
            namespace_held=self.namespace_reference is not None and self.namespace_reference.fd is not None,
            namespace_closed_after_checks=(self.namespace_reference is not None
                                          and self.namespace_reference.explicitly_closed),
            namespace_release=copy.deepcopy(self.namespace_release))

    def close(self):
        held = self.namespace_reference is not None and self.namespace_reference.fd is not None
        if self.namespace_reference is not None:
            self.namespace_reference.close()
        return dict(super().close(), observer_namespace_descriptor_closed=held,
                    namespace_close_is_release_evidence=False)
