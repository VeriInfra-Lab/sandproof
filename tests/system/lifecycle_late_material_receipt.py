"""Read-only E9 observations around an externally scheduled late receipt.

No action is replayed, manager stopped, pin closed or quota released here. The
caller must independently establish the actual late-effect window. These
observations alone cannot attest a physical delete or complete E9 acceptance.
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

from lifecycle_unload_retirement import source_record
from lifecycle_unload_runtime import (
    checked_source_records, material_source_charge, original_json,
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


class LateMaterialReceiptObserver:
    """Freeze A's Retiring plan after another owner B reuses its position.

    B may be a fresh execution or a restored execution. A restored B also needs
    its original manifest-backed files and open use pin. Verification accepts
    only A remaining Retiring or settling that same plan, while all other saved
    sources and B's running identity/reservations stay unchanged.
    """
    def __init__(self, runtime, *, session, original, occupant, deadline):
        old, new = original['original_position'], occupant['context']
        if (old['position'] != new['position']
                or old['binding']['owner'] == new['binding']['owner']
                or old['binding']['sandbox'] == new['binding']['sandbox']
                or old['binding']['vm'] == new['binding']['vm']
                or new['expected_previous'] != old['binding']['vm']):
            raise ValueError('another owner must occupy the exact original position')
        self.runtime, self.session, self.deadline = runtime, session, deadline
        self.native = copy.deepcopy(runtime.services.native_pools[0])
        self.host_config = copy.deepcopy(runtime.services.host_configs[0])
        self.original, self.occupant = copy.deepcopy(original), copy.deepcopy(occupant)
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
            old = source_record(records, self.session, self.original)
            physical = self._physical(records)
        after = database.stat()
        if (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
            raise ValueError('original database replaced during late receipt observation')
        return dict(database=(before.st_dev, before.st_ino),
            old=old, others=[row for row in records if row is not old],
            current=current, physical=physical, charge=material_source_charge(old))

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
