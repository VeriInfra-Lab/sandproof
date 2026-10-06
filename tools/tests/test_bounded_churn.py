"""Only the probe's guards/measurements; none of these tests execute KVM."""
import copy
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/server_bounded_churn.py'
SPEC = importlib.util.spec_from_file_location('bounded_churn_probe', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def settings():
    cap = dict(max_database_bytes=1024 * 1024, max_wal_bytes=1024 * 1024,
               max_shm_bytes=65536, max_online_storage_bytes=4 * 1024 * 1024,
               max_sql_rows=1000, max_sql_bytes=1024 * 1024)
    result = dict(namespace='a' * 16, database='/run/accept/db/controller.db',
                  node_state_dir='/run/accept/node', chroot_base='/run/accept/jails',
                  kernel='/opt/accept/kernel', rootfs='/opt/accept/rootfs',
                  jailer='/opt/accept/jailer', firecracker='/opt/accept/firecracker',
                  uid_base=10000, vcpu_count=1, memory_mib=512,
                  file_policy_digest=1, max_active_global=2, max_active_per_tenant=2,
                  max_disk_bytes_global=8 * 1024 * 1024, cleanup_reserve_bytes=5 * 1024 * 1024,
                  max_timeout_secs=600, expiry_scan_seconds=1,
                  data_transfer=dict(max_transfers=8, max_transfers_per_vm=4, control_reserve=1),
                  bounded_host=dict(uid_base=10000, uplink_base=0, console_policy='discard',
                                    maintenance=dict(max_host_slots=2)),
                  maintenance=dict(retention_ms=1000, capacity=cap, max_checkpoint_bytes=16384,
                                   max_tail_inputs=20, max_tail_bytes=16384))
    pool = dict(role='runtime', namespace=result['namespace'], database=result['database'],
                node_state_dir=result['node_state_dir'], chroot_base=result['chroot_base'],
                rootfs_budget_bytes=3 * 1024 * 1024, max_host_slots=2, kernel_copy_bytes=100,
                runtime_copy_bytes=200, cleanup_reserve_bytes=result['cleanup_reserve_bytes'],
                host_metadata_bytes=1024 * 1024, host_metadata_inodes=20, jail_inodes_per_slot=10)
    result['bounded_host']['physical_storage'] = dict(
        version=1, coordination_file='/run/accept/coordination', pools=[pool], public_build=None,
        filesystem_overhead_bytes=1024, filesystem_overhead_inodes=100)
    return result


def host_settings(value):
    return dict(chroot_base=value['chroot_base'], min_free_disk_bytes=32 * 1024 * 1024,
                artifacts={name: dict(path=value[name]) for name in probe.host.ARTIFACTS})


class ConfigurationTests(unittest.TestCase):
    def test_fixed_paths_and_full_profile_are_required(self):
        valid = settings()
        pool, peak, inodes = probe.validate_configuration(valid, host_settings(valid), 4, 30)
        self.assertEqual(pool['max_host_slots'], 2)
        self.assertGreater(peak, 0)
        self.assertEqual(inodes, 100)
        invalid = []
        for keys, replacement in (
            (('bounded_host', 'console_policy'), 'file'),
            (('bounded_host', 'maintenance', 'max_host_slots'), 3),
            (('max_vm_id',), 4),
            (('max_active_global',), 3),
            (('maintenance', 'retention_ms'), 30000),
            (('max_timeout_secs',), 300),
            (('database',), '/run/random/controller.db'),
        ):
            value = copy.deepcopy(valid)
            target = value
            for key in keys[:-1]:
                target = target[key]
            target[keys[-1]] = replacement
            invalid.append(value)
        value = copy.deepcopy(valid)
        del value['maintenance']['capacity']
        invalid.append(value)
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                probe.validate_configuration(value, host_settings(value), 4, 30)
        old_preflight = host_settings(valid)
        old_preflight['artifacts']['rootfs']['path'] = '/opt/other/rootfs'
        with self.assertRaises(ValueError):
            probe.validate_configuration(valid, old_preflight, 4, 30)
        old_preflight = host_settings(valid)
        old_preflight['min_free_disk_bytes'] = 1
        with self.assertRaises(ValueError):
            probe.validate_configuration(valid, old_preflight, 4, 30)

    def test_node_projection_matches_server_route_derivation_without_credentials(self):
        value = settings()
        value['credentials'] = [{'tenant': 1, 'secret': 'test-secret'}]
        node = probe.node_configuration(value)
        self.assertEqual(node['state_dir'], value['node_state_dir'])
        self.assertNotIn('node_state_dir', node)
        self.assertNotIn('credentials', node)
        self.assertEqual(node['max_vm_id'], 2**64 - 1)
        self.assertEqual(node['bounded_host']['max_data_routes_per_slot'], 4)
        self.assertEqual(node['bounded_host']['max_data_routes_global'], 8)
        self.assertEqual(node['bounded_host']['data_route_control_reserve'], 1)
        self.assertNotIn('max_data_routes_per_slot', value['bounded_host'])

    def test_missing_or_partial_rust_probe_result_is_not_success(self):
        report = dict(version=1, context_rejected=True, release_rejected=True,
                      all_actions_rejected=True, checked_actions=9)
        probe.validate_stale_report(report, 9)
        for field in report:
            bad = report.copy()
            del bad[field]
            with self.subTest(field=field), self.assertRaises(AssertionError):
                probe.validate_stale_report(bad, 9)
        for field in ('context_rejected', 'release_rejected', 'all_actions_rejected'):
            bad = report | {field: False}
            with self.assertRaises(AssertionError):
                probe.validate_stale_report(bad, 9)
        for count in (8, True, '9'):
            with self.assertRaises(AssertionError):
                probe.validate_stale_report(report | {'checked_actions': count}, 9)


class ObservationTests(unittest.TestCase):
    def test_wal_aware_reader_sees_uncheckpointed_rows_and_closes_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / 'controller.db'
            writer = sqlite3.connect(database)
            self.addCleanup(writer.close)
            writer.execute('PRAGMA journal_mode=WAL')
            writer.execute('PRAGMA wal_autocheckpoint=0')
            writer.executescript('''
                CREATE TABLE sandboxes(sandbox_id TEXT,state_blob BLOB);
                CREATE TABLE controller_checkpoint(id INTEGER,generation INTEGER,input_cut INTEGER,payload BLOB);
                CREATE TABLE controller_checkpoint_gc(id INTEGER,deleted_through INTEGER);
                CREATE TABLE controller_inputs(sequence INTEGER,input_json BLOB);
                CREATE TABLE controller_host_slots(position INTEGER,context_json BLOB);
                CREATE TABLE controller_id_highwaters(kind TEXT,scope TEXT,high_water TEXT);
                CREATE TABLE outbox(sandbox_id TEXT,action_json BLOB,id INTEGER);
            ''')
            writer.execute('INSERT INTO sandboxes VALUES(?,?)', ('sb-x-9', b'{"phase":"Running"}'))
            writer.execute('INSERT INTO controller_checkpoint VALUES(1,2,3,?)', (b'checkpoint',))
            writer.execute('INSERT INTO controller_checkpoint_gc VALUES(1,2)')
            writer.executemany('INSERT INTO controller_inputs VALUES(?,?)', [(3, b'covered'), (4, b'tail')])
            context = {'position': 1, 'binding': {'sandbox': 9, 'vm': 8}}
            writer.execute('INSERT INTO controller_host_slots VALUES(1,?)', (json.dumps(context).encode(),))
            writer.execute('INSERT INTO controller_id_highwaters VALUES(?,?,?)', ('sandbox', '', '9'))
            writer.execute('INSERT INTO outbox VALUES(?,?,1)', ('sb-x-9', b'{"accepted":"real"}'))
            writer.commit()
            original_connect = sqlite3.connect
            readers = []

            def track_reader(*args, **kwargs):
                reader = original_connect(*args, **kwargs)
                readers.append(reader)
                return reader

            with patch.object(probe.sqlite3, 'connect', side_effect=track_reader):
                snapshot = probe.database_snapshot(database)
                actions = probe.capture_actions(database, 'sb-x-9')
            self.assertEqual(len(readers), 2)
            for reader in readers:
                with self.assertRaises(sqlite3.ProgrammingError):
                    reader.execute('SELECT 1')
            self.assertEqual(snapshot['phases'], {'sb-x-9': 'Running'})
            self.assertEqual(snapshot['tail'], {'rows': 1, 'bytes': 4})
            self.assertEqual(snapshot['checkpoint']['deleted_through'], 2)
            self.assertEqual(snapshot['highwaters'], {'sandbox:': 9})
            self.assertEqual(probe.context_for(snapshot, 'sb-x-9'), context)
            self.assertEqual(actions, [{'accepted': 'real'}])
            # The observation does not keep an external reader pinned.
            self.assertEqual(writer.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()[0], 0)

    def test_budget_checks_exact_limit_and_overflow(self):
        value = settings()
        snapshot = dict(files=dict(database=1024 * 1024, wal=1024 * 1024, shm=65536),
                        checkpoint=dict(bytes=16384), tail=dict(rows=20, bytes=16384),
                        sql_payload_bytes=1024 * 1024,
                        counts=dict(sandboxes=1000, controller_inputs=10, controller_checkpoint=1,
                                    controller_host_slots=2))
        probe.assert_budget(snapshot, value)
        for section, key in (('files', 'database'), ('files', 'wal'), ('files', 'shm'),
                             ('checkpoint', 'bytes'), ('tail', 'rows'), ('tail', 'bytes'),
                             ('counts', 'sandboxes')):
            bad = copy.deepcopy(snapshot)
            bad[section][key] += 1
            with self.subTest(section=section, key=key), self.assertRaises(AssertionError):
                probe.assert_budget(bad, value)
        with self.assertRaises(AssertionError):
            probe.assert_budget(snapshot | {'sql_payload_bytes': 1024 * 1024 + 1}, value)

    def test_metadata_snapshot_rejects_links_and_detects_same_length_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            record = root / 'record.json'
            record.write_text('first')
            before = probe.tree_records([root])
            record.write_text('other')
            self.assertNotEqual(before, probe.tree_records([root]))
            (root / 'link').symlink_to(record)
            with self.assertRaisesRegex(AssertionError, 'link'):
                probe.tree_records([root])

    def test_private_evidence_never_overwrites_an_existing_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'report.json'
            probe.write_private(path, {'result': 'incomplete'})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                probe.write_private(path, {'result': 'passed'})
            self.assertEqual(json.loads(path.read_text()), {'result': 'incomplete'})


if __name__ == '__main__':
    unittest.main()
