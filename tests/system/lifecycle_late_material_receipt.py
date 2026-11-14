"""Read-only E9 observations and the original post-delete receipt gate.

The observer never stops managers or changes business state. The explicitly
selected probe reuses the gate's preparatory Host restart, observes the other
owner, then releases the original receipt. This is not late stop/delete or
complete E9 acceptance; actual execution still needs independent KVM evidence.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

from contextlib import closing
import copy
import json
from pathlib import Path
import sqlite3
import stat
import time

from lifecycle_unload_retirement import RetirementRestartProbe, source_record
from lifecycle_unload_runtime import (
    checked_source_records, gate_publish, material_source_charge, original_json,
    original_process, remaining,
)
from server_process_crash import process_start_ticks
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path


def _identity(path, *, directory=False):
    value = Path(path).lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if not kind(value.st_mode) or (not directory and value.st_nlink != 1):
        raise ValueError('exact occupant resource identity required')
    # Guest writes may update disk timestamps/content. They must not replace
    # the original inode, change ownership/mode or resize its fixed disk.
    return dict(device=value.st_dev, inode=value.st_ino, mode=value.st_mode,
        uid=value.st_uid, gid=value.st_gid,
        **({} if directory else dict(bytes=value.st_size)))


def _bounded_json(connection, table, column, key, value, limit):
    # All SQL identifiers below are internal constants, never caller input.
    where = f'FROM {table} WHERE {key}=?'
    header = connection.execute(f'SELECT length({column}) {where}', (value,)).fetchone()
    if header is None or type(header[0]) is not int or not 0 < header[0] <= limit:
        raise ValueError('bounded original occupant projection required')
    raw = connection.execute(f'SELECT {column} {where}', (value,)).fetchone()[0]
    return json.loads(raw, object_pairs_hook=unique_keys)


class SlotOccupantObserver:
    """Read-only current occupant, backing sources and resource reservations."""
    def __init__(self, runtime, *, occupant, deadline):
        self.runtime, self.deadline = runtime, deadline
        self.native = copy.deepcopy(runtime.services.native_pools[0])
        self.host_config = copy.deepcopy(runtime.services.host_configs[0])
        self.occupant = copy.deepcopy(occupant)
        self.baseline = None

    def _native(self, connection):
        native = self.native
        context, local = self.occupant['context'], self.occupant['local']
        binding = context['binding']
        if (local['sandbox'] != binding['sandbox'] or local['owner'] != binding['owner']):
            raise ValueError('original occupant local binding differs')
        ident = 'sb-' + native['namespace'] + '-' + str(local['sandbox'])
        limit = self.host_config['native']['maintenance']['capacity']['max_sql_value_bytes']
        saved_context = _bounded_json(connection, 'controller_host_slots', 'context_json',
            'position', context['position'], limit)
        state = _bounded_json(connection, 'sandboxes', 'state_blob', 'sandbox_id', ident, limit)
        credential = connection.execute('SELECT generation,digest FROM '
            'controller_data_credentials WHERE sandbox_id=?', (ident,)).fetchone()
        lifetime = connection.execute('SELECT owner_id,disk_bytes,started_at_ms,deadline_ms,'
            'max_deadline_ms,startup_deadline_ms FROM sandbox_lifetimes WHERE sandbox_id=?',
            (ident,)).fetchone()
        if (saved_context != context or state['phase'] != 'Running'
                or state['terminal'] is not False or state['owner'] != binding['owner']
                or state['generation'] != binding['generation']
                or state['current_vm'] != binding['vm']
                or credential is None or credential[0] != str(binding['generation'])
                or not isinstance(credential[1], bytes) or len(credential[1]) != 32
                or connection.execute('SELECT poisoned FROM '
                    'controller_data_credential_state WHERE id=1').fetchone() != (0,)
                or lifetime is None or lifetime[0] != 'tenant-' + str(binding['owner'])
                or any(type(n) is not int or n < 0 for n in lifetime[1:])
                or lifetime[1] <= 0):
            raise ValueError('original occupant generation, credential or reservation differs')
        return dict(context=saved_context, state=state,
            credential=(credential[0], list(credential[1])), lifetime=lifetime)

    def _physical(self, records):
        remaining(self.deadline)
        native = self.native
        source, binding = self.occupant['source'], self.occupant['context']['binding']
        resource = 'vm-' + native['namespace'] + '-' + str(binding['vm'])
        claims = Path(native['chroot_base']) / '.claims' / resource
        if (source['resource_id'] != resource or original_json(claims/'record.json', 16384) != source
                or not original_process(source)
                or process_start_ticks(source['pid']) != source['start_time_ticks']):
            raise ValueError('exact live occupant PID/start/boot required')
        namespace = _identity(Path('/run/netns')/resource)
        network = original_json(claims/'network.json', 16384)
        if (network['resource_id'] != resource or network['namespace_inode'] != namespace['inode']
                or network['namespace_device'] != namespace['device']):
            raise ValueError('original occupant network identity differs')
        group = Path(native['host_resources']['cgroup_parent'])/resource
        cgroup = _identity(group, directory=True)
        if str(source['pid']) not in (group/'cgroup.procs').read_text().split():
            raise ValueError('occupant left its original cgroup')
        disk = _identity(Path(native['chroot_base'])/Path(native['firecracker']).name
            / resource / 'root/rootfs.ext4')
        backing = None
        restored = [row for row in records if row['restore'] is not None
            and row['work']['anchor']['local']['sandbox'] == binding['sandbox']
            and row['restore']['generation'] == binding['generation']]
        if restored and 'running_backing' not in self.occupant:
            raise ValueError('original restored occupant file baseline required')
        if 'running_backing' in self.occupant:
            expected = self.occupant['running_backing']
            if (len(restored) != 1 or restored[0]['capture'] is None
                    or restored[0]['capture']['completed'] != expected['manifest']['sha256']
                    or restored[0]['restore']['activation'] is None
                    or restored[0]['restore']['material_use'] is not True):
                raise ValueError('original restored occupant backing pin required')
            backing = self.runtime._materials(restored[0]['capture']['full'], restored[0]['capture'])
            if backing != expected:
                raise ValueError('original occupant backing files changed')
        if not original_process(source) or process_start_ticks(source['pid']) != source['start_time_ticks']:
            raise ValueError('occupant exited during read-only resource observation')
        remaining(self.deadline)
        return dict(source=copy.deepcopy(source), network=namespace, cgroup=cgroup,
            disk=disk, backing=backing)

    def _snapshot(self):
        native = self.native
        if (self.runtime.services.native_pools[0] != native
                or self.runtime.services.host_configs[0] != self.host_config):
            raise ValueError('original deployment configuration changed')
        database = private_path(native['database'])
        before = database.stat()
        with closing(sqlite3.connect(database.as_uri()+'?mode=ro', uri=True,
                timeout=min(2, remaining(self.deadline)))) as connection:
            connection.execute('BEGIN')
            records = checked_source_records(connection, self.host_config, self.deadline)
            current = self._native(connection)
            physical = self._physical(records)
        after = database.stat()
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise ValueError('original database replaced during late receipt observation')
        return dict(database=(before.st_dev, before.st_ino), records=records,
            current=current, physical=physical)

    def freeze(self):
        if self.baseline is not None:
            raise ValueError('original occupant baseline cannot be refreshed')
        self.baseline = copy.deepcopy(self._snapshot())

    def verify(self):
        if self.baseline is None or self._snapshot() != self.baseline:
            raise ValueError('original occupant, backing or reservations changed')
        return dict(occupant_context=copy.deepcopy(self.baseline['current']['context']),
            occupant_source=copy.deepcopy(self.baseline['physical']['source']))


class LateMaterialReceiptObserver(SlotOccupantObserver):
    """Freeze A's Retiring plan while another owner B keeps its reused position.

    B may be fresh or restored, with its original backing files and open use pin.
    Only A remaining Retiring or settling that same plan may change.
    """
    def __init__(self, runtime, *, session, original, occupant, deadline):
        old, new = original['original_position'], occupant['context']
        if (old['position'] != new['position']
                or old['binding']['owner'] == new['binding']['owner']
                or old['binding']['sandbox'] == new['binding']['sandbox']
                or old['binding']['vm'] == new['binding']['vm']
                or new['expected_previous'] != old['binding']['vm']):
            raise ValueError('another owner must occupy the exact original position')
        super().__init__(runtime, occupant=occupant, deadline=deadline)
        self.session, self.original = session, copy.deepcopy(original)

    def _snapshot(self):
        snapshot = super()._snapshot()
        records = snapshot.pop('records')
        old = source_record(records, self.session, self.original)
        return dict(snapshot, old=old, others=[row for row in records if row is not old],
            charge=material_source_charge(old))

    def freeze(self):
        if self.baseline is not None:
            raise ValueError('late receipt baseline cannot be refreshed')
        baseline = self._snapshot()
        if (baseline['old']['retirement'] is None
                or baseline['old']['retirement']['state'] != 'retiring'
                or baseline['charge'] != baseline['old']['capture']['reserved_material_bytes']):
            raise ValueError('original fixed Retiring plan and full charge required before late receipt')
        self.baseline = copy.deepcopy(baseline)

    def verify(self):
        if self.baseline is None:
            raise ValueError('original pre-receipt baseline required')
        before, after = self.baseline, self._snapshot()
        old, current = before['old'], after['old']
        state = current['retirement']
        if (any(before[key] != after[key] for key in ('database', 'others', 'current', 'physical'))
                or {k:v for k,v in old.items() if k != 'retirement'}
                    != {k:v for k,v in current.items() if k != 'retirement'}
                or state is None or state['plan'] != old['retirement']['plan']
                or state['state'] not in ('retiring', 'settled')
                or after['charge'] != (before['charge'] if state['state'] == 'retiring' else 0)):
            raise ValueError('late receipt changed the fixed plan, current occupant or another reservation')
        return dict(state=state['state'], held_material_bytes=after['charge'],
            occupant_context=copy.deepcopy(after['current']['context']),
            occupant_source=copy.deepcopy(after['physical']['source']))


class LateMaterialReceiptProbe(RetirementRestartProbe):
    """Delay an original receipt after deletion while the other owner is live.

    Arm reuses the existing preparatory Host restart. Between the reached cut
    and settlement this probe only observes and releases that exact gate. It
    does not inject a late stop/delete or restart a manager in the held window.
    """
    def __init__(self, runtime, session, original, occupant, deadline):
        super().__init__(runtime, session, original, deadline)
        self.observer = LateMaterialReceiptObserver(runtime, session=session,
            original=original, occupant=occupant, deadline=deadline)
        self.occupant_session = occupant['session']

    def _same_host(self):
        if (self.runtime.services.processes.children['host-0'] is not self.host
                or self.host.poll() is not None):
            raise ValueError('original Host changed during delayed receipt window')
        self._configuration()

    def _workload(self, observe, previous=None):
        value = observe()
        flags = ('same_ram_and_activation', 'same_guest_process', 'same_authorization')
        if (type(value) is not dict or value.get('session') != self.occupant_session
                or any(value.get(key) is not True for key in flags)
                or any(type(value.get(key)) is not int or not 0 < value[key] < 2**64
                    for key in ('counter', 'guest_pid', 'guest_start_time_ticks'))
                or type(value.get('metadata_generation')) is not int
                or not 0 <= value['metadata_generation'] < 2**64
                or any(type(value.get(key)) is not str or len(value[key]) != 64
                    or any(c not in '0123456789abcdef' for c in value[key])
                    for key in ('ram_nonce_sha256', 'activation_nonce_sha256'))):
            raise ValueError('original other-owner guest continuity required')
        if previous is not None and (value['counter'] != previous['counter'] + 1
                or {k:v for k,v in value.items() if k != 'counter'}
                    != {k:v for k,v in previous.items() if k != 'counter'}):
            raise ValueError('other-owner guest identity changed across receipt delivery')
        return copy.deepcopy(value)

    def release_after_delete(self, successor_acceptance, observe_occupant):
        if self.recovered:
            raise ValueError('original delayed receipt gate is already completed')
        cut = self.observe_deletion(successor_acceptance)
        self._same_host()
        self.observer.freeze()
        if self.observer.baseline['old'] != cut['original']:
            raise ValueError('original SQL responsibility changed between gate and observer')
        if self.observer.verify()['state'] != 'retiring':
            raise ValueError('original receipt settled before the held-window observation')
        before = self._workload(observe_occupant)
        if self.observer.verify()['state'] != 'retiring':
            raise ValueError('original receipt was not held throughout the guest workload')
        self._same_host()
        self._unchanged()
        gate_publish(self.fd, 'command.json', dict(schema_version=1,
            run_identity=self.runtime.lease.identity, observation_sha256=cut['digest'],
            material=cut['material'], command='release'))
        self._save('command.json')
        while True:
            remaining(self.deadline)
            self._same_host()
            self._unchanged()
            settled = self.observer.verify()
            if settled['state'] == 'settled':
                break
            if settled['state'] != 'retiring':
                raise ValueError('unexpected original retirement state after gate release')
            time.sleep(min(0.05, remaining(self.deadline)))
        after = self._workload(observe_occupant, before)
        final = self.observer.verify()
        self._same_host()
        self._unchanged()
        if final != settled:
            raise ValueError('original settled receipt or occupant identity changed')
        self.recovered = True
        return dict(phase=cut['observation']['phase'], material=cut['material'],
            original_retiring_material_bytes=cut['original']['capture']['reserved_material_bytes'],
            settled_material_bytes=final['held_material_bytes'],
            occupant_context=final['occupant_context'], occupant_source=final['occupant_source'],
            occupant_before=before, occupant_after=after,
            receipt_delayed_after_actual_delete=True, same_plan_settled=True,
            same_host_manager_during_receipt_window=True,
            late_stop_or_delete_not_exercised=True)
