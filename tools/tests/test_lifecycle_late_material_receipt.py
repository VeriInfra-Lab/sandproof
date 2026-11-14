"""Real read-only SQLite plus controlled source/physical providers; no VM effects."""
import copy
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_late_material_receipt as late
from tools.tests.test_lifecycle_unload_retirement import fixture, charge


class LateMaterialFixture(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.database = root/'db'
        self.records, self.original, _, _ = fixture()
        self.records[0]['retirement']['state'] = 'retiring'
        old = dict(position=0, expected_previous=None,
            binding=dict(owner=1, sandbox=1, generation=1, vm=1))
        context = dict(position=0, expected_previous=1,
            binding=dict(owner=2, sandbox=2, generation=1, vm=2))
        self.original['original_position'] = old
        self.occupant = dict(session='other-owner-session', context=context,
            local=dict(owner=2, sandbox=2), source=dict(resource_id='vm-original-2',
                pid=42, start_time_ticks=17, boot_id='original-boot'))
        native = dict(database=str(self.database), namespace='original',
            bounded_host=dict(maintenance=dict(capacity=dict(max_sql_value_bytes=16384))))
        self.runtime = SimpleNamespace(services=SimpleNamespace(native_pools=[native],
            host_configs=[{}], processes=Mock()))
        self.state = dict(owner=2, phase='Running', terminal=False, generation=1, current_vm=2)
        with sqlite3.connect(self.database) as connection:
            connection.executescript('CREATE TABLE controller_host_slots(position,context_json);'
                'CREATE TABLE sandboxes(sandbox_id,state_blob);'
                'CREATE TABLE controller_data_credentials(sandbox_id,generation,digest);'
                'CREATE TABLE controller_data_credential_state(id,poisoned);'
                'CREATE TABLE sandbox_lifetimes(sandbox_id,owner_id,disk_bytes,started_at_ms,'
                'deadline_ms,max_deadline_ms,startup_deadline_ms);')
            connection.execute('INSERT INTO controller_host_slots VALUES(0,?)', (json.dumps(context),))
            connection.execute('INSERT INTO sandboxes VALUES(?,?)', ('sb-original-2',json.dumps(self.state)))
            connection.execute('INSERT INTO controller_data_credentials VALUES(?,?,?)',
                ('sb-original-2','1',b'x'*32))
            connection.execute('INSERT INTO controller_data_credential_state VALUES(1,0)')
            connection.execute('INSERT INTO sandbox_lifetimes VALUES(?,?,?,?,?,?,?)',
                ('sb-original-2','tenant-2',4096,1,100,200,20))
        # Existing strict source accounting and root-owned physical readers have
        # their own tests. These fixtures isolate this observer's composition.
        self.start_patch(patch.object(late, 'private_path', side_effect=Path))
        self.start_patch(patch.object(late, 'checked_source_records',
            side_effect=lambda *_: copy.deepcopy(self.records)))
        self.start_patch(patch.object(late, 'material_source_charge', side_effect=charge))
        self.physical = dict(source=copy.deepcopy(self.occupant['source']),
            disk=dict(device=1,inode=10), network=dict(device=2,inode=20),
            cgroup=dict(device=3,inode=30),backing=None)
        self.probe = late.LateMaterialReceiptObserver(self.runtime, session='original',
            original=self.original, occupant=self.occupant, deadline=time.monotonic()+20)
        self.reader = self.start_patch(patch.object(self.probe, '_physical',
            side_effect=lambda _: copy.deepcopy(self.physical)))

    def start_patch(self, patcher):
        self.addCleanup(patcher.stop)
        return patcher.start()

    def update(self, query, values=()):
        with sqlite3.connect(self.database) as connection:
            connection.execute(query, values)


class LateMaterialReceiptTests(LateMaterialFixture):
    def test_pending_then_same_plan_settled_preserves_other_owner_without_manager_calls(self):
        before = self.database.read_bytes()
        self.probe.freeze()
        self.assertEqual(self.probe.verify()['held_material_bytes'], 100)
        self.assertEqual(self.database.read_bytes(), before)
        self.records[0]['retirement']['state'] = 'settled'
        result = self.probe.verify()
        self.assertEqual(result['state'], 'settled')
        self.assertEqual(result['held_material_bytes'], 0)
        self.assertEqual(result['occupant_context'], self.occupant['context'])
        self.runtime.services.processes.assert_not_called()
        self.assertEqual(self.runtime.services.processes.mock_calls, [])
        with self.assertRaises(ValueError):
            self.probe.freeze()

    def test_retired_missing_or_replanned_original_cannot_refresh_baseline(self):
        self.probe.freeze()
        original = copy.deepcopy(self.records)
        for fault in ('missing','settlement-plan','reopened-pin','other-charge','old-history'):
            self.records[:] = copy.deepcopy(original)
            if fault == 'missing': self.records.pop(0)
            elif fault == 'settlement-plan': self.records[0]['retirement']['plan']['bytes'] = [8]
            elif fault == 'reopened-pin': self.records[0]['restore']['material_use'] = True
            elif fault == 'other-charge': self.records[1]['capture']['reserved_material_bytes'] += 1
            elif fault == 'old-history': self.records[0]['capture']['completed'] = [8]*32
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                self.probe.verify()
        self.records[:] = original
        self.assertEqual(self.probe.verify()['state'], 'retiring')

    def test_current_sql_generation_credential_slot_and_reservation_changes_are_rejected(self):
        self.probe.freeze()
        for key,value in [('generation',2),('current_vm',3),('owner',1),('phase','Destroyed'),
                          ('terminal',True)]:
            self.update('UPDATE sandboxes SET state_blob=?', (json.dumps(dict(self.state,**{key:value})),))
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.probe.verify()
        self.update('UPDATE sandboxes SET state_blob=?', (json.dumps(self.state),))
        for query,reset in [
            ("UPDATE controller_data_credentials SET digest=zeroblob(32)",
             'UPDATE controller_data_credentials SET digest=?'),
            ('UPDATE sandbox_lifetimes SET disk_bytes=2048','UPDATE sandbox_lifetimes SET disk_bytes=4096'),
            ('UPDATE controller_data_credential_state SET poisoned=1',
             'UPDATE controller_data_credential_state SET poisoned=0'),
            ('DELETE FROM controller_host_slots','INSERT INTO controller_host_slots VALUES(0,?)'),
        ]:
            self.update(query)
            with self.subTest(query=query), self.assertRaises(ValueError):
                self.probe.verify()
            values = (b'x'*32,) if 'digest=?' in reset else (
                (json.dumps(self.occupant['context']),) if 'VALUES' in reset else ())
            self.update(reset,values)
        self.assertEqual(self.probe.verify()['state'], 'retiring')

    def test_changed_physical_process_disk_network_or_cgroup_is_never_success(self):
        self.probe.freeze()
        original = copy.deepcopy(self.physical)
        for key in ('source','disk','network','cgroup','backing'):
            self.physical = copy.deepcopy(original)
            self.physical[key] = dict(replacement=True)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.probe.verify()

    def test_requires_actual_reuse_by_other_owner_and_pre_receipt_retiring_state(self):
        for field,value in [('position',1),('expected_previous',9)]:
            occupant = copy.deepcopy(self.occupant)
            occupant['context'][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                late.LateMaterialReceiptObserver(self.runtime, session='original',
                    original=self.original,occupant=occupant,deadline=time.monotonic()+10)
        for field in ('owner','sandbox','vm'):
            occupant = copy.deepcopy(self.occupant)
            occupant['context']['binding'][field] = self.original['original_position']['binding'][field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                late.LateMaterialReceiptObserver(self.runtime, session='original',
                    original=self.original,occupant=occupant,deadline=time.monotonic()+10)
        with self.assertRaises(ValueError):
            self.probe.verify()
        self.records[0]['retirement']['state'] = 'settled'
        with self.assertRaises(ValueError):
            self.probe.freeze()

    def test_database_replacement_or_configuration_switch_cannot_move_the_baseline(self):
        self.probe.freeze()
        replacement = self.database.with_name('replacement')
        replacement.write_bytes(self.database.read_bytes())
        os.replace(replacement,self.database)
        with self.assertRaises(ValueError):
            self.probe.verify()
        self.runtime.services.native_pools[0]['namespace'] = 'another'
        with self.assertRaises(ValueError):
            self.probe.verify()


class LinuxLateMaterialPhysicalReaderTests(LateMaterialFixture):
    def test_physical_reader_uses_real_process_and_files_with_controlled_claim_provider(self):
        root = Path(self.directory.name)
        source = dict(resource_id='vm-original-2',pid=os.getpid(),
            start_time_ticks=late.process_start_ticks(os.getpid()),
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
        native = self.probe.native
        native.update(chroot_base=str(root/'v'),firecracker='/fixed/firecracker',
            host_resources=dict(cgroup_parent=str(root/'groups')))
        self.runtime.services.native_pools[0] = copy.deepcopy(native)
        self.probe.occupant['source'] = source
        claims = root/'v/.claims'/source['resource_id']
        claims.mkdir(parents=True)
        (claims/'record.json').write_text(json.dumps(source))
        namespace = root/'namespace'
        namespace.touch()
        identity = late._identity
        stamp = identity(namespace)
        network = dict(resource_id=source['resource_id'],
            namespace_device=stamp['device'],namespace_inode=stamp['inode'])
        (claims/'network.json').write_text(json.dumps(network))
        group = root/'groups'/source['resource_id']
        group.mkdir(parents=True)
        (group/'cgroup.procs').write_text(str(source['pid']))
        disk = root/'v/firecracker'/source['resource_id']/'root/rootfs.ext4'
        disk.parent.mkdir(parents=True)
        disk.write_bytes(b'fixed test disk')
        # Controlled claim provider avoids claiming privileged root ownership.
        # Process checks and the disk/cgroup/namespace inode reads remain real.
        with patch.object(late,'original_json',side_effect=lambda path,_:json.loads(path.read_text())), \
                patch.object(late,'_identity',side_effect=lambda path,**kw:identity(
                    namespace if Path(path).parent == Path('/run/netns') else path,**kw)):
            read = lambda:late.LateMaterialReceiptObserver._physical(self.probe,self.records)
            observed = read()
            self.assertEqual(observed['source'],source)
            self.assertEqual(observed['disk']['inode'],disk.stat().st_ino)
            with patch.object(late,'process_start_ticks',return_value=None), self.assertRaises(ValueError):
                read()
            (group/'cgroup.procs').write_text('')
            with self.assertRaises(ValueError): read()
            (group/'cgroup.procs').write_text(str(source['pid']))
            (claims/'network.json').write_text(json.dumps(dict(network,namespace_inode=0)))
            with self.assertRaises(ValueError): read()
            (claims/'network.json').write_text(json.dumps(network))
            restored = copy.deepcopy(self.records[0])
            restored['work']['anchor']['local']['sandbox'] = 2
            restored['restore'].update(generation=1,material_use=True)
            self.records.append(restored)
            with self.assertRaises(ValueError): read()
            backing = dict(manifest=dict(sha256=restored['capture']['completed']))
            self.probe.occupant['running_backing'] = backing
            self.runtime._materials = Mock(return_value=copy.deepcopy(backing))
            self.assertEqual(read()['backing'],backing)
            restored['restore']['material_use'] = False
            with self.assertRaises(ValueError): read()
            restored['restore']['material_use'] = True
            self.runtime._materials.return_value = dict(replacement=True)
            with self.assertRaises(ValueError): read()


if __name__ == '__main__':
    unittest.main()
